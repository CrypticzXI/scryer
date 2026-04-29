#![recursion_limit = "256"]

mod common;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, DownloadSubmissionRepository, EpisodeScopedMediaFile,
    EpisodeUpdate, InsertMediaFileInput, MediaFileAnalysis, MediaFileRepository, PendingRelease,
    ReleaseDecision, ScopedExternalId, ShowRepository, TitleEpisodeProgressSummary, TitleMediaFile,
    TitleMediaSizeSummary, TitleQualitySummary, TitleRepository, WantedItem, WantedItemRepository,
    start_background_download_delete_poller,
};
use scryer_domain::{
    Collection, CollectionType, DomainEventPayload, DomainEventStream, DomainExternalIds,
    DownloadFailedEventData, Episode, EpisodeType, ExternalId, Id, ImportCompletedEventData,
    MediaFacet, MediaPathUpdate, MediaUpdateType, NewDomainEvent, ReleaseBlocklistedEventData,
    Title, TitleContextSnapshot,
};
use scryer_infrastructure::{
    FileSystemLibraryRenamer, SettingDefinitionSeed, SqliteCatalogStore, SqliteLibraryStateStore,
    SqliteWorkflowStore,
};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashMap;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use common::{TestContext, load_fixture};

/// Execute a GraphQL operation directly against the schema, without going
/// through the HTTP test server.  This gives full control over what data
/// (e.g. `User`) is attached to the request.
async fn schema_exec(ctx: &TestContext, query: &str, user: Option<scryer_domain::User>) -> Value {
    let mut req = async_graphql::Request::new(query);
    if let Some(u) = user {
        req = req.data(u);
    }
    let resp = ctx.schema.execute(req).await;
    serde_json::to_value(&resp).expect("serialize gql response")
}

/// Helper to execute a GraphQL query and return the parsed JSON body.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

/// Assert no GraphQL errors in response body.
fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn set_rename_collision_policy(ctx: &TestContext, scope: &str, policy: &str) {
    let body = gql(
        ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameCollisionPolicy
          }
        }
        "#,
        json!({
            "input": {
                "scope": scope,
                "renameCollisionPolicy": policy
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["updateMediaSettings"]["renameCollisionPolicy"],
        policy
    );
}

struct FailingMediaFileRepo {
    inner: SqliteLibraryStateStore,
    fail_file_id: String,
}

#[async_trait]
impl MediaFileRepository for FailingMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        <SqliteLibraryStateStore as MediaFileRepository>::insert_media_file(&self.inner, input)
            .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        <SqliteLibraryStateStore as MediaFileRepository>::link_file_to_episode(
            &self.inner,
            file_id,
            episode_id,
        )
        .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        <SqliteLibraryStateStore as MediaFileRepository>::list_media_files_for_title(
            &self.inner,
            title_id,
        )
        .await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        <SqliteLibraryStateStore as MediaFileRepository>::list_live_media_files_for_episode_ids(
            &self.inner,
            title_id,
            episode_ids,
        )
        .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        <SqliteLibraryStateStore as MediaFileRepository>::list_title_media_size_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        <SqliteLibraryStateStore as MediaFileRepository>::list_title_quality_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        <SqliteLibraryStateStore as MediaFileRepository>::list_title_episode_progress_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        <SqliteLibraryStateStore as MediaFileRepository>::update_media_file_analysis(
            &self.inner,
            file_id,
            analysis,
        )
        .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        <SqliteLibraryStateStore as MediaFileRepository>::update_media_file_source_signature(
            &self.inner,
            file_id,
            size_bytes,
            source_signature_scheme,
            source_signature_value,
        )
        .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        if file_id == self.fail_file_id {
            return Err(AppError::Repository(format!(
                "injected media file path failure for {file_id} -> {file_path}"
            )));
        }

        <SqliteLibraryStateStore as MediaFileRepository>::update_media_file_path(
            &self.inner,
            file_id,
            file_path,
        )
        .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        <SqliteLibraryStateStore as MediaFileRepository>::mark_scan_failed(
            &self.inner,
            file_id,
            error,
        )
        .await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        <SqliteLibraryStateStore as MediaFileRepository>::get_media_file_by_id(&self.inner, file_id)
            .await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        <SqliteLibraryStateStore as MediaFileRepository>::get_media_file_by_path(
            &self.inner,
            file_path,
        )
        .await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        <SqliteLibraryStateStore as MediaFileRepository>::delete_media_file(&self.inner, file_id)
            .await
    }
}

struct FailingShowRepo {
    inner: SqliteCatalogStore,
    fail_collection_id: String,
    fail_path: String,
}

#[async_trait]
impl ShowRepository for FailingShowRepo {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        self.inner.list_collections_for_title(title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        self.inner.list_collection_external_ids(collection_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        self.inner.list_collections_for_titles(title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        self.inner.get_collection_by_id(collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        self.inner
            .get_collection_by_ordered_path(ordered_path)
            .await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        self.inner.create_collection(collection).await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        if collection_id == self.fail_collection_id
            && update.ordered_path.as_deref() == Some(self.fail_path.as_str())
        {
            return Err(AppError::Repository(format!(
                "injected collection path failure for {collection_id}"
            )));
        }

        self.inner.update_collection(collection_id, update).await
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        self.inner
            .update_collection_interstitial_movie(collection_id, interstitial_movie)
            .await
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        self.inner
            .update_collection_specials_movies(collection_id, specials_movies)
            .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .update_interstitial_season_episode(collection_id, season_episode)
            .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        self.inner
            .set_collection_episodes_monitored(collection_id, monitored)
            .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        self.inner.delete_collection(collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        self.inner.delete_collections_for_title(title_id).await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        self.inner.list_episodes_for_collection(collection_id).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        self.inner.list_episodes_for_title(title_id).await
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        self.inner.list_episode_external_ids(episode_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        self.inner.get_episode_by_id(episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.inner.create_episode(episode).await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        self.inner.update_episode(episode_id, update).await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        self.inner.delete_episode(episode_id).await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        self.inner.delete_episodes_for_title(title_id).await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.inner
            .find_episode_by_title_and_numbers(title_id, season_number, episode_number)
            .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.inner
            .find_episode_by_title_and_absolute_number(title_id, absolute_number)
            .await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_inclusive: &str,
        end_inclusive: &str,
    ) -> AppResult<Vec<scryer_domain::CalendarEpisode>> {
        self.inner
            .list_episodes_in_date_range(start_inclusive, end_inclusive)
            .await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::PrimaryCollectionSummary>> {
        self.inner
            .list_primary_collection_summaries(title_ids)
            .await
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        self.inner
            .replace_anibridge_scoped_external_ids_for_title(title_id, collection_ids, episode_ids)
            .await
    }
}

/// Helper to add a title and return the title ID.
async fn add_test_title(ctx: &TestContext, name: &str, facet: &str) -> String {
    let tvdb_id = match facet {
        "movie" => "123456",
        "series" | "anime" => "345678",
        _ => "123456",
    };
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id name } } }"#,
        json!({
            "input": {
                "name": name,
                "facet": facet,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb_id }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn seed_typed_settings_definitions(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_api_key".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_username".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_password".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.languages".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.auto_download_on_import".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_series".into(),
                data_type: "number".into(),
                default_value_json: "240".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.search_interval_hours".into(),
                data_type: "number".into(),
                default_value_json: "6".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_ai_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_machine_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_series".into(),
                data_type: "number".into(),
                default_value_json: "90".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_max_offset_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.upgrade_cooldown_hours".into(),
                data_type: "number".into(),
                default_value_json: "24".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.same_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "120".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.cross_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "30".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.forced_upgrade_delta_bypass".into(),
                data_type: "number".into(),
                default_value_json: "400".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.poll_interval_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.sync_interval_seconds".into(),
                data_type: "number".into(),
                default_value_json: "3600".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.batch_size".into(),
                data_type: "number".into(),
                default_value_json: "50".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.delay_profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "history.keep_forever".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "history.retention_days".into(),
                data_type: "number".into(),
                default_value_json: "180".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profile_id".into(),
                data_type: "string".into(),
                default_value_json: "\"4k\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.scoring_persona".into(),
                data_type: "string".into(),
                default_value_json: "\"Balanced\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "audio.required_languages".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "audio.required_languages.title_override".into(),
                data_type: "json".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "download_client.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "indexer.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"{title} ({year}) - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.series.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season:2}E{episode:2} - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.anime.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}\""
                        .into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.filler_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.recap_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_specials".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.inter_season_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_filler_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.movie".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.cert_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.key_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("settings definitions should seed");
}

async fn mount_smg_mocks(ctx: &TestContext, fixture_path: &str) {
    let fixture = load_fixture(fixture_path);
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;
}

async fn create_series_scan_title(
    ctx: &TestContext,
    media_root: &std::path::Path,
    name: &str,
    extra_tags: Vec<String>,
) -> (Title, Collection) {
    let mut tags = vec![format!("scryer:root-folder:{}", media_root.display())];
    tags.extend(extra_tags);

    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags,
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
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
        runtime_minutes: Some(24),
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
    };
    let title = ctx
        .catalog
        .create(title)
        .await
        .expect("create series title");

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("10".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .catalog
        .create_collection(collection)
        .await
        .expect("create season collection");

    (title, collection)
}

async fn create_catalog_title(
    ctx: &TestContext,
    name: &str,
    facet: MediaFacet,
    external_ids: Vec<ExternalId>,
    tags: Vec<String>,
    monitored: bool,
) -> Title {
    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        facet,
        monitored,
        tags,
        external_ids,
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Original overview".to_string()),
        poster_url: Some("https://example.com/old-poster.jpg".to_string()),
        poster_source_url: None,
        banner_url: Some("https://example.com/old-banner.jpg".to_string()),
        banner_source_url: None,
        background_url: Some("https://example.com/old-background.jpg".to_string()),
        background_source_url: None,
        sort_title: Some(name.to_string()),
        slug: Some("old-slug".to_string()),
        imdb_id: Some("tt0000001".to_string()),
        runtime_minutes: Some(100),
        genres: vec!["Drama".to_string()],
        content_status: Some("ended".to_string()),
        language: Some("eng".to_string()),
        first_aired: Some("2020-01-01".to_string()),
        network: Some("Old Network".to_string()),
        studio: Some("Old Studio".to_string()),
        country: Some("usa".to_string()),
        aliases: vec!["Legacy Alias".to_string()],
        tagged_aliases: vec![],
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now()),
        min_availability: None,
        digital_release_date: Some("2020-01-01".to_string()),
        folder_path: None,
    };

    ctx.catalog.create(title).await.expect("create title")
}

#[derive(Debug, PartialEq, Eq)]
struct SeriesMonitoringSummary {
    title_monitored: bool,
    collection_monitored: bool,
    episode_monitored: bool,
    wanted_count: i64,
}

async fn create_series_monitoring_fixture(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
) -> (Title, Collection, Episode) {
    let title = create_catalog_title(
        ctx,
        name,
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: Some("Pilot episode".to_string()),
            tvdb_id: Some(format!("{tvdb_id}01")),
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    (title, collection, episode)
}

async fn series_monitoring_summary(
    ctx: &TestContext,
    title_id: &str,
    collection_id: &str,
    episode_id: &str,
) -> SeriesMonitoringSummary {
    let title = ctx
        .catalog
        .get_by_id(title_id)
        .await
        .expect("load title")
        .expect("title");
    let collection = ctx
        .catalog
        .get_collection_by_id(collection_id)
        .await
        .expect("load collection")
        .expect("collection");
    let episode = ctx
        .catalog
        .get_episode_by_id(episode_id)
        .await
        .expect("load episode")
        .expect("episode");
    let wanted_count = ctx
        .library_state
        .count_wanted_items(scryer_application::WantedItemsQuery {
            title_id: Some(title_id.to_string()),
            ..scryer_application::WantedItemsQuery::default()
        })
        .await
        .expect("count wanted items");

    SeriesMonitoringSummary {
        title_monitored: title.monitored,
        collection_monitored: collection.monitored,
        episode_monitored: episode.monitored,
        wanted_count,
    }
}

async fn activity_kinds_for_title(ctx: &TestContext, title_id: &str) -> Vec<String> {
    let body = gql(ctx, "{ activityEvents { kind titleId } }", json!({})).await;
    assert_no_errors(&body);

    body["data"]["activityEvents"]
        .as_array()
        .expect("activity events array")
        .iter()
        .filter(|event| event["titleId"] == title_id)
        .filter_map(|event| event["kind"].as_str())
        .map(str::to_string)
        .collect()
}

async fn create_series_scan_episode(
    ctx: &TestContext,
    title: &Title,
    collection: &Collection,
    season_number: &str,
    episode_number: &str,
    label: &str,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some(season_number.to_string()),
        episode_label: Some(label.to_string()),
        title: Some(format!("Episode {episode_number}")),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.catalog
        .create_episode(episode)
        .await
        .expect("create episode")
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_uses_media_file_rows() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Rename Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "91001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9100103".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Rename Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("[SubsPlease] Rename Preview Show - 03 (1080p).mkv");
    std::fs::write(&file_path, b"anime-preview").expect("write preview file");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            noop
            conflicts
            errors
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["noop"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(0));
    assert_eq!(plan["errors"].as_i64(), Some(0));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], Value::Null);
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            season_dir
                .join("Rename Preview Show - S01E03 (012) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
    assert_eq!(item["reasonCode"], "rename_move");
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_interstitial_uses_season_zero_numbering() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Festival Saga",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "92001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_zero_dir = media_root.path().join("Festival Saga").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let file_path = season_zero_dir.join("Festival.Saga.Movie.Special.1080p.mkv");
    std::fs::write(&file_path, b"anime-interstitial").expect("write interstitial file");

    let interstitial = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9200103".to_string(),
                name: "Festival Film".to_string(),
                slug: "festival-film".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Festival special".to_string(),
                poster_url: "https://example.com/festival-film.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 95,
                sort_title: "Festival Film".to_string(),
                imdb_id: "tt9200103".to_string(),
                genres: vec!["Fantasy".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], json!(interstitial.id));
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            season_zero_dir
                .join("Festival Saga - S00E03 (3) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_anime_updates_media_files_and_only_interstitial_collections() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Apply Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "93001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9300101".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root.path().join("Anime Apply Show").join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let regular_file_path = season_dir.join("Anime.Apply.Show.Episode.One.1080p.mkv");
    std::fs::write(&regular_file_path, b"anime-apply-episode").expect("write regular file");

    let regular_file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: regular_file_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert regular file");
    ctx.db
        .link_file_to_episode(&regular_file_id, &episode.id)
        .await
        .expect("link regular file");

    let season_zero_dir = media_root.path().join("Anime Apply Show").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let interstitial_file_path = season_zero_dir.join("Anime.Apply.Show.Movie.Special.1080p.mkv");
    std::fs::write(&interstitial_file_path, b"anime-apply-interstitial")
        .expect("write interstitial file");

    let interstitial_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(interstitial_file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9300103".to_string(),
                name: "Pilot Movie".to_string(),
                slug: "pilot-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Pilot side story".to_string(),
                poster_url: "https://example.com/pilot-movie.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 90,
                sort_title: "Pilot Movie".to_string(),
                imdb_id: "tt9300103".to_string(),
                genres: vec!["Adventure".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let interstitial_file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: interstitial_file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 2);

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 2);
    assert_eq!(result.failed, 0);

    let expected_regular_path = season_dir
        .join("Anime Apply Show - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let expected_interstitial_path = season_zero_dir
        .join("Anime Apply Show - S00E03 (3) - 1080p.mkv")
        .to_string_lossy()
        .to_string();

    let updated_regular_file = ctx
        .library_state
        .get_media_file_by_id(&regular_file_id)
        .await
        .expect("load updated regular media file")
        .expect("regular media file");
    let updated_interstitial_file = ctx
        .library_state
        .get_media_file_by_id(&interstitial_file_id)
        .await
        .expect("load updated interstitial media file")
        .expect("interstitial media file");
    let refreshed_season_collection = ctx
        .catalog
        .get_collection_by_id(&season_collection.id)
        .await
        .expect("load season collection")
        .expect("season collection");
    let refreshed_interstitial_collection = ctx
        .catalog
        .get_collection_by_id(&interstitial_collection.id)
        .await
        .expect("load interstitial collection")
        .expect("interstitial collection");

    assert_eq!(updated_regular_file.file_path, expected_regular_path);
    assert_eq!(
        updated_interstitial_file.file_path,
        expected_interstitial_path
    );
    assert_eq!(refreshed_season_collection.ordered_path, None);
    assert_eq!(
        refreshed_interstitial_collection.ordered_path,
        Some(expected_interstitial_path.clone())
    );
    assert!(std::path::Path::new(&expected_regular_path).exists());
    assert!(std::path::Path::new(&expected_interstitial_path).exists());
    assert!(!regular_file_path.exists());
    assert!(!interstitial_file_path.exists());
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_stays_collection_based() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Regression Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Regression Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let file_path = movie_dir.join("Regression.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"movie-rename-preview").expect("write movie file");

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], json!(collection.id));
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            movie_dir
                .join("Regression Movie (2024) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_movies_updates_collection_and_media_file_paths() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Movie Apply Sync (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Movie Apply Sync (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Movie.Apply.Sync.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"movie-apply-sync").expect("write movie file");

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 8192,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert_eq!(
        preview.items[0].media_file_id.as_deref(),
        Some(file_id.as_str())
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);
    assert_eq!(result.failed, 0);

    let expected_path = movie_dir
        .join("Movie Apply Sync (2024) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let updated_collection = ctx
        .catalog
        .get_collection_by_id(&collection.id)
        .await
        .expect("load movie collection")
        .expect("movie collection");
    let updated_file = ctx
        .library_state
        .get_media_file_by_id(&file_id)
        .await
        .expect("load movie media file")
        .expect("movie media file");

    assert_eq!(
        updated_collection.ordered_path.as_deref(),
        Some(expected_path.as_str())
    );
    assert_eq!(updated_file.file_path, expected_path);
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "anime", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9500103".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Tracked Collision Anime")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let source_path = season_dir.join("[SubsPlease] Tracked Collision Anime - 03 (1080p).mkv");
    std::fs::write(&source_path, b"tracked-collision-source").expect("write source file");
    let destination_path = season_dir.join("Tracked Collision Anime - S01E03 (012) - 1080p.mkv");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert source media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: owning_title.id,
            file_path: destination_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert tracked destination");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "movie", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Tracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Tracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"tracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Tracked Collision Movie (2024) - 1080p.mkv");

    ctx.catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: owning_title.id,
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(destination_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create tracked destination collection");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_multi_episode_file_uses_episode_range() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Range Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");
    let episode_one =
        create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    let episode_two =
        create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let season_dir = media_root
        .path()
        .join("Range Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Range.Preview.Show.S01E01-E02.1080p.mkv");
    std::fs::write(&file_path, b"anime-range-preview").expect("write preview file");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode_one.id)
        .await
        .expect("link first episode");
    ctx.db
        .link_file_to_episode(&file_id, &episode_two.id)
        .await
        .expect("link second episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              mediaFileId
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["mediaFileId"], json!(file_id));
    assert_eq!(plan["items"][0]["writeAction"], "move");
    assert_eq!(
        plan["items"][0]["proposedPath"],
        json!(
            season_dir
                .join("Range Preview Show - S01E01-02 (01-02) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_untracked_existing_target_does_not_emit_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "movie", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Untracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Untracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Untracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"untracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Untracked Collision Movie (2024) - 1080p.mkv");
    std::fs::write(&destination_path, b"untracked-movie-destination")
        .expect("write untracked destination");

    ctx.catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn apply_media_rename_for_anime_rolls_back_when_media_file_update_fails() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Media Rollback",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "98001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9800101".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Anime Media Rollback")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let source_path = season_dir.join("Anime.Media.Rollback.Episode.One.1080p.mkv");
    std::fs::write(&source_path, b"anime-media-rollback").expect("write source file");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_media_files(std::sync::Arc::new(FailingMediaFileRepo {
            inner: ctx.library_state.clone(),
            fail_file_id: file_id.clone(),
        }))
    });

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 1);
    assert!(
        result
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let expected_path = season_dir
        .join("Anime Media Rollback - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let item = &result.items[0];
    assert_eq!(item.status.as_str(), "failed");
    assert_eq!(item.reason_code, "db_update_failed");
    assert_eq!(
        item.final_path.as_deref(),
        Some(source_path.to_string_lossy().as_ref())
    );
    assert!(
        item.error_message
            .as_deref()
            .is_some_and(|message| message.contains("rollback succeeded"))
    );

    let stored = ctx
        .library_state
        .get_media_file_by_id(&file_id)
        .await
        .expect("load media file")
        .expect("media file present");
    assert_eq!(stored.file_path, source_path.to_string_lossy().to_string());
    assert!(source_path.exists());
    assert!(!std::path::Path::new(&expected_path).exists());
}

#[tokio::test]
async fn apply_media_rename_for_anime_interstitial_rolls_back_when_collection_update_fails() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Interstitial Rollback",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "99001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_zero_dir = media_root
        .path()
        .join("Anime Interstitial Rollback")
        .join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let source_path = season_zero_dir.join("Anime.Interstitial.Rollback.Movie.Special.1080p.mkv");
    std::fs::write(&source_path, b"anime-interstitial-rollback").expect("write source file");
    let expected_path = season_zero_dir
        .join("Anime Interstitial Rollback - S00E03 (3) - 1080p.mkv")
        .to_string_lossy()
        .to_string();

    let interstitial = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9900103".to_string(),
                name: "Rollback Movie".to_string(),
                slug: "rollback-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Rollback special".to_string(),
                poster_url: "https://example.com/rollback-movie.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 88,
                sort_title: "Rollback Movie".to_string(),
                imdb_id: "tt9900103".to_string(),
                genres: vec!["Adventure".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial media file");

    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_shows(std::sync::Arc::new(FailingShowRepo {
            inner: SqliteCatalogStore::new(&ctx.db),
            fail_collection_id: interstitial.id.clone(),
            fail_path: expected_path.clone(),
        }))
    });

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 1);

    let item = &result.items[0];
    assert_eq!(item.status.as_str(), "failed");
    assert_eq!(item.reason_code, "db_update_failed");
    assert_eq!(
        item.final_path.as_deref(),
        Some(source_path.to_string_lossy().as_ref())
    );
    assert!(
        item.error_message
            .as_deref()
            .is_some_and(|message| message.contains("rollback succeeded"))
    );

    let stored_file = ctx
        .library_state
        .get_media_file_by_id(&file_id)
        .await
        .expect("load interstitial media file")
        .expect("interstitial media file");
    let stored_collection = ctx
        .catalog
        .get_collection_by_id(&interstitial.id)
        .await
        .expect("load interstitial collection")
        .expect("interstitial collection");
    assert_eq!(
        stored_file.file_path,
        source_path.to_string_lossy().to_string()
    );
    assert_eq!(
        stored_collection.ordered_path,
        Some(source_path.to_string_lossy().to_string())
    );
    assert!(source_path.exists());
    assert!(!std::path::Path::new(&expected_path).exists());
}

// ---------------------------------------------------------------------------
// Basic connectivity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_get_returns_non_500() {
    let ctx = TestContext::new().await;
    let resp = ctx
        .http_client()
        .get(format!("{}/graphql", ctx.app_url))
        .send()
        .await
        .unwrap();
    // GET on a POST-only endpoint — should not crash
    assert_ne!(resp.status().as_u16(), 500);
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_introspection_query_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { queryType { name } } }", json!({})).await;
    assert_eq!(body["data"]["__schema"]["queryType"]["name"], "QueryRoot");
}

#[tokio::test]
async fn graphql_introspection_mutation_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { mutationType { name } } }", json!({})).await;
    assert_eq!(
        body["data"]["__schema"]["mutationType"]["name"],
        "MutationRoot"
    );
}

#[tokio::test]
async fn graphql_introspection_query_root_uses_semantic_search_and_browse_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "QueryRoot") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"searchReleases"));
    assert!(!names.contains(&"searchIndexers"));
    assert!(!names.contains(&"searchIndexersEpisode"));
    assert!(!names.contains(&"searchIndexersForTitle"));
    assert!(!names.contains(&"searchIndexersForEpisode"));
    assert!(!names.contains(&"titleCollections"));
    assert!(!names.contains(&"collectionEpisodes"));
    assert!(!names.contains(&"titleMediaFiles"));
    assert!(names.contains(&"wantedItem"));
    assert!(names.contains(&"pendingRelease"));
    assert!(names.contains(&"downloadHistory"));
}

#[tokio::test]
async fn graphql_introspection_exposes_collection_search_input_on_search_releases() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          searchReleasesInput: __type(name: "SearchReleasesInput") {
            inputFields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["searchReleasesInput"]["inputFields"]
        .as_array()
        .expect("should have input fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"titleId"));
    assert!(names.contains(&"collectionId"));
    assert!(names.contains(&"season"));
    assert!(names.contains(&"episode"));
}

#[tokio::test]
async fn graphql_search_releases_rejects_collection_and_episode_inputs_together() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        query SearchReleases($input: SearchReleasesInput!) {
          searchReleases(input: $input) { title }
        }
        "#,
        json!({
            "input": {
                "titleId": "title-1",
                "collectionId": "collection-1",
                "season": "1",
                "episode": "1"
            }
        }),
    )
    .await;

    let errors = body["errors"].as_array().expect("expected graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("expected graphql error message");
    assert!(message.contains("collection searches cannot include season or episode"));
}

#[tokio::test]
async fn graphql_introspection_exposes_typed_settings_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields { name }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields { name }
          }
          subtitleSettings: __type(name: "SubtitleSettingsPayload") {
            fields { name }
          }
          acquisitionSettings: __type(name: "AcquisitionSettingsPayload") {
            fields { name }
          }
          generalSettings: __type(name: "GeneralSettingsPayload") {
            fields { name }
          }
          mediaSettings: __type(name: "MediaSettingsPayload") {
            fields { name }
          }
          libraryPaths: __type(name: "LibraryPathsPayload") {
            fields { name }
          }
          serviceSettings: __type(name: "ServiceSettingsPayload") {
            fields { name }
          }
          qualityProfileSettings: __type(name: "QualityProfileSettingsPayload") {
            fields { name }
          }
          qualityProfileCriteriaPayload: __type(name: "QualityProfileCriteriaPayload") {
            fields { name }
          }
          qualityProfileCriteriaInput: __type(name: "QualityProfileCriteriaInput") {
            inputFields { name }
          }
          updateSubtitleSettingsInput: __type(name: "UpdateSubtitleSettingsInput") {
            inputFields { name }
          }
          updateGeneralSettingsInput: __type(name: "UpdateGeneralSettingsInput") {
            inputFields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let query_names: Vec<&str> = query_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(query_names.contains(&"subtitleSettings"));
    assert!(query_names.contains(&"acquisitionSettings"));
    assert!(query_names.contains(&"generalSettings"));
    assert!(query_names.contains(&"mediaSettings"));
    assert!(query_names.contains(&"libraryPaths"));
    assert!(query_names.contains(&"serviceSettings"));
    assert!(query_names.contains(&"qualityProfileSettings"));
    assert!(query_names.contains(&"downloadClientRouting"));
    assert!(query_names.contains(&"indexerRouting"));
    assert!(!query_names.contains(&"convenienceSettings"));
    assert!(!query_names.contains(&"adminSettings"));

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation_names: Vec<&str> = mutation_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(mutation_names.contains(&"updateSubtitleSettings"));
    assert!(mutation_names.contains(&"updateAcquisitionSettings"));
    assert!(mutation_names.contains(&"updateGeneralSettings"));
    assert!(mutation_names.contains(&"updateMediaSettings"));
    assert!(mutation_names.contains(&"updateLibraryPaths"));
    assert!(mutation_names.contains(&"updateServiceSettings"));
    assert!(mutation_names.contains(&"saveQualityProfileSettings"));
    assert!(mutation_names.contains(&"updateDownloadClientRouting"));
    assert!(mutation_names.contains(&"updateIndexerRouting"));
    assert!(!mutation_names.contains(&"updateQualityProfileFacetPersona"));
    assert!(!mutation_names.contains(&"saveAdminSettings"));

    let subtitle_fields = body["data"]["subtitleSettings"]["fields"]
        .as_array()
        .expect("SubtitleSettingsPayload should expose fields");
    let subtitle_names: Vec<&str> = subtitle_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(subtitle_names.contains(&"languages"));
    assert!(!subtitle_names.contains(&"openSubtitlesUsername"));
    assert!(!subtitle_names.contains(&"hasOpenSubtitlesApiKey"));
    assert!(!subtitle_names.contains(&"hasOpenSubtitlesPassword"));

    let subtitle_input_fields = body["data"]["updateSubtitleSettingsInput"]["inputFields"]
        .as_array()
        .expect("UpdateSubtitleSettingsInput should expose input fields");
    let subtitle_input_names: Vec<&str> = subtitle_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!subtitle_input_names.contains(&"openSubtitlesUsername"));
    assert!(!subtitle_input_names.contains(&"openSubtitlesPassword"));
    assert!(!subtitle_input_names.contains(&"openSubtitlesApiKey"));

    let acquisition_fields = body["data"]["acquisitionSettings"]["fields"]
        .as_array()
        .expect("AcquisitionSettingsPayload should expose fields");
    let acquisition_names: Vec<&str> = acquisition_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(acquisition_names.contains(&"pollIntervalSeconds"));
    assert!(acquisition_names.contains(&"batchSize"));

    let general_fields = body["data"]["generalSettings"]["fields"]
        .as_array()
        .expect("GeneralSettingsPayload should expose fields");
    let general_names: Vec<&str> = general_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(general_names.contains(&"keepHistoryForever"));
    assert!(general_names.contains(&"historyRetentionDays"));

    let media_fields = body["data"]["mediaSettings"]["fields"]
        .as_array()
        .expect("MediaSettingsPayload should expose fields");
    let media_names: Vec<&str> = media_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(media_names.contains(&"libraryPath"));
    assert!(media_names.contains(&"rootFolders"));
    assert!(media_names.contains(&"requiredAudioLanguages"));
    assert!(media_names.contains(&"renameTemplate"));

    let library_fields = body["data"]["libraryPaths"]["fields"]
        .as_array()
        .expect("LibraryPathsPayload should expose fields");
    let library_names: Vec<&str> = library_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(library_names.contains(&"moviePath"));
    assert!(library_names.contains(&"seriesPath"));
    assert!(library_names.contains(&"animePath"));

    let service_fields = body["data"]["serviceSettings"]["fields"]
        .as_array()
        .expect("ServiceSettingsPayload should expose fields");
    let service_names: Vec<&str> = service_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(service_names.contains(&"tlsCertPath"));
    assert!(service_names.contains(&"tlsKeyPath"));

    let quality_profile_settings_fields = body["data"]["qualityProfileSettings"]["fields"]
        .as_array()
        .expect("QualityProfileSettingsPayload should expose fields");
    let quality_profile_settings_names: Vec<&str> = quality_profile_settings_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(quality_profile_settings_names.contains(&"globalScoringPersona"));
    assert!(quality_profile_settings_names.contains(&"categoryPersonaSelections"));

    let criteria_payload_fields = body["data"]["qualityProfileCriteriaPayload"]["fields"]
        .as_array()
        .expect("QualityProfileCriteriaPayload should expose fields");
    let criteria_payload_names: Vec<&str> = criteria_payload_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!criteria_payload_names.contains(&"requiredAudioLanguages"));
    assert!(!criteria_payload_names.contains(&"scoringPersona"));
    assert!(!criteria_payload_names.contains(&"facetPersonaOverrides"));
    assert!(!criteria_payload_names.contains(&"atmosPreferred"));

    let criteria_input_fields = body["data"]["qualityProfileCriteriaInput"]["inputFields"]
        .as_array()
        .expect("QualityProfileCriteriaInput should expose inputFields");
    let criteria_input_names: Vec<&str> = criteria_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!criteria_input_names.contains(&"requiredAudioLanguages"));
    assert!(!criteria_input_names.contains(&"scoringPersona"));
    assert!(!criteria_input_names.contains(&"facetPersonaOverrides"));
    assert!(!criteria_input_names.contains(&"atmosPreferred"));

    let general_input_fields = body["data"]["updateGeneralSettingsInput"]["inputFields"]
        .as_array()
        .expect("UpdateGeneralSettingsInput should expose inputFields");
    let general_input_names: Vec<&str> = general_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(general_input_names.contains(&"keepHistoryForever"));
    assert!(general_input_names.contains(&"historyRetentionDays"));
}

#[tokio::test]
async fn graphql_typed_media_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            libraryPath
            rootFolders { path isDefault }
            requiredAudioLanguages
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({
          "input": {
            "scope": "anime",
            "rootFolders": [
              { "path": "/library/anime-main", "isDefault": true },
              { "path": "/library/anime-archive", "isDefault": false }
            ],
            "requiredAudioLanguages": ["eng", "jpn"],
            "renameTemplate": "{title} [{quality}].{ext}",
            "renameCollisionPolicy": "replace_if_better",
            "renameMissingMetadataPolicy": "skip",
            "fillerPolicy": "skip_filler",
            "recapPolicy": "skip_recap",
            "monitorSpecials": true,
            "interSeasonMovies": false,
            "monitorFillerMovies": true,
            "nfoWriteOnImport": true,
            "plexmatchWriteOnImport": true
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let updated = &update["data"]["updateMediaSettings"];
    assert_eq!(updated["scope"], "anime");
    assert_eq!(updated["libraryPath"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["path"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["isDefault"], true);
    assert_eq!(updated["requiredAudioLanguages"][0], "eng");
    assert_eq!(updated["requiredAudioLanguages"][1], "jpn");
    assert_eq!(updated["renameTemplate"], "{title} [{quality}].{ext}");
    assert_eq!(updated["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(updated["renameMissingMetadataPolicy"], "skip");
    assert_eq!(updated["fillerPolicy"], "skip_filler");
    assert_eq!(updated["recapPolicy"], "skip_recap");
    assert_eq!(updated["monitorSpecials"], true);
    assert_eq!(updated["interSeasonMovies"], false);
    assert_eq!(updated["monitorFillerMovies"], true);
    assert_eq!(updated["nfoWriteOnImport"], true);
    assert_eq!(updated["plexmatchWriteOnImport"], true);

    let read = gql(
        &ctx,
        r#"
        query MediaSettings($scope: ContentScopeValue!) {
          mediaSettings(scope: $scope) {
            scope
            libraryPath
            rootFolders { path isDefault }
            requiredAudioLanguages
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({ "scope": "anime" }),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["mediaSettings"];
    assert_eq!(settings["scope"], "anime");
    assert_eq!(settings["libraryPath"], "/library/anime-main");
    assert_eq!(settings["rootFolders"][1]["path"], "/library/anime-archive");
    assert_eq!(settings["requiredAudioLanguages"][0], "eng");
    assert_eq!(settings["requiredAudioLanguages"][1], "jpn");
    assert_eq!(settings["renameTemplate"], "{title} [{quality}].{ext}");
    assert_eq!(settings["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(settings["renameMissingMetadataPolicy"], "skip");
    assert_eq!(settings["fillerPolicy"], "skip_filler");
    assert_eq!(settings["recapPolicy"], "skip_recap");
    assert_eq!(settings["monitorSpecials"], true);
    assert_eq!(settings["interSeasonMovies"], false);
    assert_eq!(settings["monitorFillerMovies"], true);
    assert_eq!(settings["nfoWriteOnImport"], true);
    assert_eq!(settings["plexmatchWriteOnImport"], true);
}

#[tokio::test]
async fn graphql_typed_library_paths_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/mnt/storage/movies",
            "seriesPath": "/mnt/storage/series",
            "animePath": "/mnt/storage/anime"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateLibraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );

    let read = gql(
        &ctx,
        r#"
        query LibraryPaths {
          libraryPaths {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["libraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["seriesPath"],
        "/mnt/storage/series"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["animePath"],
        "/mnt/storage/anime"
    );
}

#[tokio::test]
async fn graphql_typed_service_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
          updateServiceSettings(input: $input) {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({
          "input": {
            "tlsCertPath": "/etc/scryer/tls.crt",
            "tlsKeyPath": "/etc/scryer/tls.key"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateServiceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );

    let read = gql(
        &ctx,
        r#"
        query ServiceSettings {
          serviceSettings {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["serviceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );
    assert_eq!(
        read["data"]["serviceSettings"]["tlsKeyPath"],
        "/etc/scryer/tls.key"
    );
}

#[tokio::test]
async fn graphql_typed_subtitle_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "subtitles.opensubtitles_api_key",
            None,
            json!("smg-managed-key").to_string(),
            "test",
            None,
        )
        .await
        .expect("subtitle api key should seed");
    let update = gql(
        &ctx,
        r#"
        mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
          updateSubtitleSettings(input: $input) {
            enabled
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "languages": [
              { "code": "eng", "hearingImpaired": true, "forced": false },
              { "code": "spa", "hearingImpaired": false, "forced": true }
            ],
            "autoDownloadOnImport": true,
            "minimumScoreSeries": 255,
            "minimumScoreMovie": 85,
            "searchIntervalHours": 12,
            "includeAiTranslated": true,
            "includeMachineTranslated": false,
            "syncEnabled": true,
            "syncThresholdSeries": 91,
            "syncThresholdMovie": 74,
            "syncMaxOffsetSeconds": 48
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query SubtitleSettings {
          subtitleSettings {
            enabled
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["subtitleSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["autoDownloadOnImport"], true);
    assert_eq!(settings["minimumScoreSeries"], 255);
    assert_eq!(settings["minimumScoreMovie"], 85);
    assert_eq!(settings["searchIntervalHours"], 12);
    assert_eq!(settings["includeAiTranslated"], true);
    assert_eq!(settings["includeMachineTranslated"], false);
    assert_eq!(settings["syncEnabled"], true);
    assert_eq!(settings["syncThresholdSeries"], 91);
    assert_eq!(settings["syncThresholdMovie"], 74);
    assert_eq!(settings["syncMaxOffsetSeconds"], 48);
    assert_eq!(settings["languages"][0]["code"], "eng");
    assert_eq!(settings["languages"][0]["hearingImpaired"], true);
    assert_eq!(settings["languages"][1]["code"], "spa");
    assert_eq!(settings["languages"][1]["forced"], true);
}

#[tokio::test]
async fn graphql_typed_acquisition_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
          updateAcquisitionSettings(input: $input) {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "upgradeCooldownHours": 18,
            "sameTierMinDelta": 140,
            "crossTierMinDelta": 35,
            "forcedUpgradeDeltaBypass": 420,
            "pollIntervalSeconds": 45,
            "syncIntervalSeconds": 1800,
            "batchSize": 25
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query AcquisitionSettings {
          acquisitionSettings {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["acquisitionSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["upgradeCooldownHours"], 18);
    assert_eq!(settings["sameTierMinDelta"], 140);
    assert_eq!(settings["crossTierMinDelta"], 35);
    assert_eq!(settings["forcedUpgradeDeltaBypass"], 420);
    assert_eq!(settings["pollIntervalSeconds"], 45);
    assert_eq!(settings["syncIntervalSeconds"], 1800);
    assert_eq!(settings["batchSize"], 25);
}

#[tokio::test]
async fn graphql_typed_general_settings_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], false);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 180);
}

#[tokio::test]
async fn graphql_typed_general_settings_round_trip_and_forever_preserves_days() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let first_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 45
          }
        }),
    )
    .await;
    assert_no_errors(&first_update);
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );

    let forever_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": true,
            "historyRetentionDays": 0
          }
        }),
    )
    .await;
    assert_no_errors(&forever_update);
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["keepHistoryForever"],
        true
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], true);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 45);
}

#[tokio::test]
async fn graphql_typed_general_settings_rejects_invalid_days() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let body = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 0
          }
        }),
    )
    .await;

    assert!(
        body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected validation errors: {body}"
    );
}

#[tokio::test]
async fn graphql_delay_profiles_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let upsert = gql(
        &ctx,
        r#"
        mutation UpsertDelayProfile($input: DelayProfileInput!) {
          upsertDelayProfile(input: $input) {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({
          "input": {
            "id": "balanced-delay",
            "name": "Balanced Delay",
            "usenetDelayMinutes": 30,
            "torrentDelayMinutes": 90,
            "preferredProtocol": "usenet",
            "minAgeMinutes": 15,
            "bypassScoreThreshold": 320,
            "appliesToFacets": ["movie", "series"],
            "tags": ["4k", "hdr"],
            "priority": 5,
            "enabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&upsert);
    assert_eq!(upsert["data"]["upsertDelayProfile"]["id"], "balanced-delay");
    assert_eq!(
        upsert["data"]["upsertDelayProfile"]["appliesToFacets"][1],
        "series"
    );

    let read = gql(
        &ctx,
        r#"
        query DelayProfiles {
          delayProfiles {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    let profile = &read["data"]["delayProfiles"][0];
    assert_eq!(profile["id"], "balanced-delay");
    assert_eq!(profile["name"], "Balanced Delay");
    assert_eq!(profile["usenetDelayMinutes"], 30);
    assert_eq!(profile["torrentDelayMinutes"], 90);
    assert_eq!(profile["preferredProtocol"], "usenet");
    assert_eq!(profile["minAgeMinutes"], 15);
    assert_eq!(profile["bypassScoreThreshold"], 320);
    assert_eq!(profile["appliesToFacets"][0], "movie");
    assert_eq!(profile["appliesToFacets"][1], "series");
    assert_eq!(profile["tags"][0], "4k");
    assert_eq!(profile["priority"], 5);
    assert_eq!(profile["enabled"], true);

    let delete = gql(
        &ctx,
        r#"
        mutation DeleteDelayProfile($input: DeleteDelayProfileInput!) {
          deleteDelayProfile(input: $input) {
            id
          }
        }
        "#,
        json!({
          "input": { "id": "balanced-delay" }
        }),
    )
    .await;
    assert_no_errors(&delete);
    assert_eq!(delete["data"]["deleteDelayProfile"]["id"], "balanced-delay");
}

#[tokio::test]
async fn graphql_quality_profile_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            globalProfileId
            globalScoringPersona
            profiles {
              id
              name
              criteria {
                qualityTiers
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
              inheritsGlobal
            }
          }
        }
        "#,
        json!({
          "input": {
            "profiles": [
              {
                "id": "custom-audio",
                "name": "Custom Audio",
                "criteria": {
                  "qualityTiers": ["2160P", "1080P"],
                  "archivalQuality": "2160P",
                  "allowUnknownQuality": false,
                  "sourceAllowlist": [],
                  "sourceBlocklist": [],
                  "videoCodecAllowlist": [],
                  "videoCodecBlocklist": [],
                  "audioCodecAllowlist": [],
                  "audioCodecBlocklist": [],
                  "dolbyVisionAllowed": true,
                  "detectedHdrAllowed": true,
                  "preferRemux": false,
                  "allowBdDisk": true,
                  "allowUpgrades": true,
                  "scoringOverrides": {},
                  "cutoffTier": null,
                  "minScoreToGrab": null
                }
              }
            ],
            "globalProfileId": "custom-audio",
            "globalScoringPersona": "Audiophile",
            "categorySelections": [
              {
                "scope": "movie",
                "profileId": "custom-audio",
                "inheritGlobal": false
              },
              {
                "scope": "series",
                "profileId": null,
                "inheritGlobal": true
              }
            ],
            "categoryPersonaSelections": [
              {
                "scope": "anime",
                "persona": "Compatible",
                "inheritGlobal": false
              }
            ],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["globalProfileId"],
        "custom-audio"
    );
    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["globalScoringPersona"],
        "Audiophile"
    );
    let anime_persona_selection =
        update["data"]["saveQualityProfileSettings"]["categoryPersonaSelections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|selection| selection["scope"] == "anime")
            .unwrap();
    assert_eq!(anime_persona_selection["overridePersona"], "Compatible");
    assert_eq!(anime_persona_selection["effectivePersona"], "Compatible");
    assert_eq!(anime_persona_selection["inheritsGlobal"], false);

    let read = gql(
        &ctx,
        r#"
        query QualityProfileSettings {
          qualityProfileSettings {
            globalProfileId
            globalScoringPersona
            profiles {
              id
              criteria {
                qualityTiers
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
              inheritsGlobal
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["qualityProfileSettings"];
    assert_eq!(settings["globalProfileId"], "custom-audio");
    assert_eq!(settings["globalScoringPersona"], "Audiophile");
    let movie_selection = settings["categorySelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["scope"] == "movie")
        .unwrap();
    assert_eq!(movie_selection["overrideProfileId"], "custom-audio");
    assert_eq!(movie_selection["inheritsGlobal"], false);

    let anime_persona_selection = settings["categoryPersonaSelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["scope"] == "anime")
        .unwrap();
    assert_eq!(anime_persona_selection["overridePersona"], "Compatible");
    assert_eq!(anime_persona_selection["effectivePersona"], "Compatible");
    assert_eq!(anime_persona_selection["inheritsGlobal"], false);
}

#[tokio::test]
async fn graphql_quality_profile_settings_updates_category_persona_selection_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let seed = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            profiles {
              id
            }
          }
        }
        "#,
        json!({
          "input": {
            "profiles": [
              {
                "id": "custom-audio",
                "name": "Custom Audio",
                "criteria": {
                  "qualityTiers": ["2160P", "1080P"],
                  "archivalQuality": "2160P",
                  "allowUnknownQuality": false,
                  "sourceAllowlist": [],
                  "sourceBlocklist": [],
                  "videoCodecAllowlist": [],
                  "videoCodecBlocklist": [],
                  "audioCodecAllowlist": [],
                  "audioCodecBlocklist": [],
                  "dolbyVisionAllowed": true,
                  "detectedHdrAllowed": true,
                  "preferRemux": false,
                  "allowBdDisk": true,
                  "allowUpgrades": true,
                  "scoringOverrides": {},
                  "cutoffTier": null,
                  "minScoreToGrab": null
                }
              }
            ],
            "globalProfileId": null,
            "globalScoringPersona": "Balanced",
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&seed);

    let update = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            globalScoringPersona
            profiles {
              id
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
              inheritsGlobal
            }
          }
        }
        "#,
        json!({
          "input": {
            "profiles": [],
            "globalProfileId": null,
            "globalScoringPersona": "Balanced",
            "categorySelections": [],
            "categoryPersonaSelections": [
              {
                "scope": "anime",
                "persona": "Compatible",
                "inheritGlobal": false
              }
            ],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["globalScoringPersona"],
        "Balanced"
    );
    let anime_override = update["data"]["saveQualityProfileSettings"]["categoryPersonaSelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["scope"] == "anime")
        .unwrap();
    assert_eq!(anime_override["overridePersona"], "Compatible");
    assert_eq!(anime_override["effectivePersona"], "Compatible");
    assert_eq!(anime_override["inheritsGlobal"], false);
}

#[tokio::test]
async fn graphql_typed_routing_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update_download = gql(
        &ctx,
        r#"
        mutation UpdateDownloadClientRouting($input: UpdateDownloadClientRoutingInput!) {
          updateDownloadClientRouting(input: $input) {
            clientId
            enabled
            category
            recentQueuePriority
            olderQueuePriority
            removeCompleted
            removeFailed
          }
        }
        "#,
        json!({
          "input": {
            "scope": "movie",
            "entries": [
              {
                "clientId": "client-a",
                "enabled": true,
                "category": "movies",
                "recentQueuePriority": "high",
                "olderQueuePriority": "low",
                "removeCompleted": true,
                "removeFailed": false
              }
            ]
          }
        }),
    )
    .await;
    assert_no_errors(&update_download);
    assert_eq!(
        update_download["data"]["updateDownloadClientRouting"][0]["clientId"],
        "client-a"
    );

    let update_indexer = gql(
        &ctx,
        r#"
        mutation UpdateIndexerRouting($input: UpdateIndexerRoutingInput!) {
          updateIndexerRouting(input: $input) {
            indexerId
            enabled
            categories
            priority
          }
        }
        "#,
        json!({
          "input": {
            "scope": "anime",
            "entries": [
              {
                "indexerId": "indexer-a",
                "enabled": true,
                "categories": ["5070", "2000"],
                "priority": 3
              }
            ]
          }
        }),
    )
    .await;
    assert_no_errors(&update_indexer);
    assert_eq!(
        update_indexer["data"]["updateIndexerRouting"][0]["indexerId"],
        "indexer-a"
    );

    let read = gql(
        &ctx,
        r#"
        query TypedRouting {
          downloadClientRouting(scope: movie) {
            clientId
            category
            recentQueuePriority
          }
          indexerRouting(scope: anime) {
            indexerId
            categories
            priority
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["downloadClientRouting"][0]["clientId"],
        "client-a"
    );
    assert_eq!(
        read["data"]["downloadClientRouting"][0]["category"],
        "movies"
    );
    assert_eq!(read["data"]["indexerRouting"][0]["indexerId"], "indexer-a");
    assert_eq!(read["data"]["indexerRouting"][0]["priority"], 3);
}

#[tokio::test]
async fn graphql_introspection_lists_title_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "TitlePayload") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();
    assert!(names.contains(&"id"), "TitlePayload should have id field");
    assert!(
        names.contains(&"name"),
        "TitlePayload should have name field"
    );
    assert!(
        names.contains(&"facet"),
        "TitlePayload should have facet field"
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_core_graph_relationship_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          title: __type(name: "TitlePayload") { fields { name } }
          collection: __type(name: "CollectionPayload") { fields { name } }
          episode: __type(name: "EpisodePayload") { fields { name } }
          queueItem: __type(name: "DownloadQueueItemPayload") { fields { name } }
          mediaFile: __type(name: "TitleMediaFilePayload") { fields { name } }
          wantedItem: __type(name: "WantedItemPayload") { fields { name } }
          releaseDecision: __type(name: "ReleaseDecisionPayload") { fields { name } }
          pendingRelease: __type(name: "PendingReleasePayload") { fields { name } }
          pendingReleaseStatus: __type(name: "PendingReleaseStatusValue") { enumValues { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let title_fields: Vec<&str> = body["data"]["title"]["fields"]
        .as_array()
        .expect("title fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(title_fields.contains(&"downloadQueueItems"));

    let collection_fields: Vec<&str> = body["data"]["collection"]["fields"]
        .as_array()
        .expect("collection fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(collection_fields.contains(&"title"));
    assert!(collection_fields.contains(&"episodes"));

    let episode_fields: Vec<&str> = body["data"]["episode"]["fields"]
        .as_array()
        .expect("episode fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(episode_fields.contains(&"parentTitle"));
    assert!(episode_fields.contains(&"collection"));
    assert!(episode_fields.contains(&"wantedItem"));
    assert!(episode_fields.contains(&"mediaFiles"));

    let queue_item_fields: Vec<&str> = body["data"]["queueItem"]["fields"]
        .as_array()
        .expect("queue item fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(queue_item_fields.contains(&"title"));

    let media_file_fields: Vec<&str> = body["data"]["mediaFile"]["fields"]
        .as_array()
        .expect("media file fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(media_file_fields.contains(&"title"));
    assert!(media_file_fields.contains(&"episode"));

    let wanted_item_fields: Vec<&str> = body["data"]["wantedItem"]["fields"]
        .as_array()
        .expect("wanted item fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(wanted_item_fields.contains(&"title"));
    assert!(wanted_item_fields.contains(&"collection"));
    assert!(wanted_item_fields.contains(&"episode"));
    assert!(wanted_item_fields.contains(&"releaseDecisions"));
    assert!(wanted_item_fields.contains(&"pendingReleases"));

    let release_decision_fields: Vec<&str> = body["data"]["releaseDecision"]["fields"]
        .as_array()
        .expect("release decision fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(release_decision_fields.contains(&"title"));
    assert!(release_decision_fields.contains(&"wantedItem"));

    let pending_release_fields: Vec<&str> = body["data"]["pendingRelease"]["fields"]
        .as_array()
        .expect("pending release fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(pending_release_fields.contains(&"title"));
    assert!(pending_release_fields.contains(&"wantedItem"));

    let pending_release_status_names: Vec<&str> =
        body["data"]["pendingReleaseStatus"]["enumValues"]
            .as_array()
            .expect("pending release status values")
            .iter()
            .filter_map(|value| value["name"].as_str())
            .collect();
    assert_eq!(
        pending_release_status_names,
        vec![
            "waiting",
            "standby",
            "processing",
            "grabbed",
            "superseded",
            "expired",
            "dismissed"
        ]
    );
}

#[tokio::test]
async fn graphql_traverses_core_graph_relationships() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = Title {
        id: Id::new().0,
        name: "Graph Traversal Show".to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Traversal coverage".to_string()),
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: Some(24),
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
    };
    let title = ctx.catalog.create(title).await.expect("create title");

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .catalog
        .create_collection(collection)
        .await
        .expect("create collection");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: Some("Episode overview".to_string()),
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .catalog
        .create_episode(episode)
        .await
        .expect("create episode");

    let file_path = media_root
        .path()
        .join("Graph.Traversal.Show.S01E01.1080p.WEB-DL.mkv");
    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4_096,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(120),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let wanted_item = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        episode_id: Some(episode.id.clone()),
        collection_id: Some(collection.id.clone()),
        season_number: Some("1".to_string()),
        media_type: "episode".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 1,
        baseline_date: None,
        status: scryer_application::WantedStatus::Wanted,
        grabbed_release: None,
        current_score: Some(120),
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: "2026-03-20T00:00:00Z".to_string(),
        updated_at: "2026-03-20T00:00:00Z".to_string(),
    };
    ctx.library_state
        .upsert_wanted_item(&wanted_item)
        .await
        .expect("seed wanted item");

    let decision = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: wanted_item.id.clone(),
        title_id: title.id.clone(),
        release_title: "Graph Traversal Show S01E01 1080p WEB-DL".to_string(),
        release_url: Some("https://example.invalid/release".to_string()),
        release_size_bytes: Some(8_192),
        decision_code: "accepted".to_string(),
        candidate_score: 140,
        current_score: Some(120),
        score_delta: Some(20),
        explanation_json: None,
        created_at: "2026-03-20T00:05:00Z".to_string(),
    };
    ctx.db
        .insert_release_decision(&decision)
        .await
        .expect("seed release decision");

    let pending_release = PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted_item.id.clone(),
        title_id: title.id.clone(),
        release_title: "Graph Traversal Show S01E01 1080p Delay Hold".to_string(),
        release_url: Some("https://example.invalid/pending".to_string()),
        source_kind: None,
        release_size_bytes: Some(16_384),
        release_score: 135,
        scoring_log_json: None,
        indexer_source: Some("test-indexer".to_string()),
        release_guid: Some("pending-guid".to_string()),
        added_at: "2026-03-20T00:06:00Z".to_string(),
        delay_until: "2026-03-20T01:06:00Z".to_string(),
        status: scryer_application::PendingReleaseStatus::Waiting,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
    };
    ctx.db
        .insert_pending_release(&pending_release)
        .await
        .expect("seed pending release");

    let body = gql(
        &ctx,
        r#"
        query CoreGraph($titleId: String!, $collectionId: String!, $episodeId: String!, $wantedItemId: String!, $pendingReleaseId: String!) {
          title(id: $titleId) {
            id
            downloadQueueItems {
              id
            }
            collections {
              id
              title { id }
              episodes {
                id
                parentTitle { id }
                collection { id }
                wantedItem { id }
                mediaFiles {
                  id
                  title { id }
                  episode { id }
                }
              }
            }
            mediaFiles {
              id
              title { id }
              episode {
                id
                parentTitle { id }
              }
            }
            wantedItems {
              id
              title { id }
              collection { id }
              episode { id }
              pendingReleases {
                id
                status
                title { id }
                wantedItem { id }
              }
              releaseDecisions(limit: 10) {
                id
                wantedItem { id }
                title { id }
              }
            }
            releaseDecisions(limit: 10) {
              id
              wantedItem { id }
              title { id }
            }
          }
          collection(id: $collectionId) {
            id
            title { id }
          }
          episode(id: $episodeId) {
            id
            parentTitle { id }
            collection { id }
            wantedItem { id }
            mediaFiles { id }
          }
          wantedItem(id: $wantedItemId) {
            id
            title { id }
            collection { id }
            episode { id }
            pendingReleases {
              id
              status
              title { id }
              wantedItem { id }
            }
            releaseDecisions(limit: 10) { id }
          }
          pendingRelease(id: $pendingReleaseId) {
            id
            status
            title { id }
            wantedItem { id }
          }
        }
        "#,
        json!({
            "titleId": title.id,
            "collectionId": collection.id,
            "episodeId": episode.id,
            "wantedItemId": wanted_item.id,
            "pendingReleaseId": pending_release.id,
        }),
    )
    .await;
    assert_no_errors(&body);

    let title_data = &body["data"]["title"];
    assert_eq!(title_data["downloadQueueItems"], json!([]));
    assert_eq!(title_data["collections"][0]["title"]["id"], title.id);
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["parentTitle"]["id"],
        title.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["collection"]["id"],
        collection.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["wantedItem"]["id"],
        wanted_item.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["mediaFiles"][0]["id"],
        file_id
    );
    assert_eq!(title_data["mediaFiles"][0]["title"]["id"], title.id);
    assert_eq!(title_data["mediaFiles"][0]["episode"]["id"], episode.id);
    assert_eq!(title_data["wantedItems"][0]["title"]["id"], title.id);
    assert_eq!(
        title_data["wantedItems"][0]["collection"]["id"],
        collection.id
    );
    assert_eq!(title_data["wantedItems"][0]["episode"]["id"], episode.id);
    assert_eq!(
        title_data["wantedItems"][0]["pendingReleases"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        title_data["wantedItems"][0]["pendingReleases"][0]["status"],
        "waiting"
    );
    assert_eq!(
        title_data["wantedItems"][0]["releaseDecisions"][0]["id"],
        decision.id
    );
    assert_eq!(
        title_data["releaseDecisions"][0]["wantedItem"]["id"],
        wanted_item.id
    );

    assert_eq!(body["data"]["collection"]["title"]["id"], title.id);
    assert_eq!(body["data"]["episode"]["parentTitle"]["id"], title.id);
    assert_eq!(body["data"]["episode"]["collection"]["id"], collection.id);
    assert_eq!(body["data"]["episode"]["wantedItem"]["id"], wanted_item.id);
    assert_eq!(body["data"]["episode"]["mediaFiles"][0]["id"], file_id);
    assert_eq!(body["data"]["wantedItem"]["title"]["id"], title.id);
    assert_eq!(
        body["data"]["wantedItem"]["collection"]["id"],
        collection.id
    );
    assert_eq!(body["data"]["wantedItem"]["episode"]["id"], episode.id);
    assert_eq!(
        body["data"]["wantedItem"]["pendingReleases"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        body["data"]["wantedItem"]["releaseDecisions"][0]["id"],
        decision.id
    );
    assert_eq!(body["data"]["pendingRelease"]["id"], pending_release.id);
    assert_eq!(body["data"]["pendingRelease"]["status"], "waiting");
    assert_eq!(body["data"]["pendingRelease"]["title"]["id"], title.id);
    assert_eq!(
        body["data"]["pendingRelease"]["wantedItem"]["id"],
        wanted_item.id
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_queue_and_source_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queueItem: __type(name: "DownloadQueueItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          queueState: __type(name: "DownloadQueueStateValue") {
            enumValues { name }
          }
          sourceKind: __type(name: "DownloadSourceKindValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["queueItem"]["fields"]
        .as_array()
        .expect("DownloadQueueItemPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("state")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("state")["type"]["ofType"]["name"],
        "DownloadQueueStateValue"
    );
    assert_eq!(field("importStatus")["type"]["name"], "ImportStatusValue");
    assert_eq!(
        field("importErrorCode")["type"]["name"],
        "ImportErrorCodeValue"
    );
    assert_eq!(
        field("trackedState")["type"]["name"],
        "TrackedDownloadStateValue"
    );
    assert_eq!(
        field("trackedStatus")["type"]["name"],
        "TrackedDownloadStatusValue"
    );
    assert_eq!(
        field("trackedMatchType")["type"]["name"],
        "TitleMatchTypeValue"
    );

    let queue_states = body["data"]["queueState"]["enumValues"]
        .as_array()
        .expect("DownloadQueueStateValue should expose enum values");
    let queue_state_names: Vec<&str> = queue_states
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(queue_state_names.contains(&"import_pending"));
    assert!(!queue_state_names.contains(&"importpending"));

    let source_kinds = body["data"]["sourceKind"]["enumValues"]
        .as_array()
        .expect("DownloadSourceKindValue should expose enum values");
    let source_kind_names: Vec<&str> = source_kinds
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        source_kind_names,
        vec!["nzbFile", "nzbUrl", "torrentFile", "magnetUri"]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_queue_action_payloads() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          actionPayload: __type(name: "DownloadQueueActionPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          actionKind: __type(name: "DownloadQueueActionKindValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation_field = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };

    for field_name in [
        "queueManualImport",
        "ignoreTrackedDownload",
        "markTrackedDownloadFailed",
        "retryTrackedDownloadImport",
        "assignTrackedDownloadTitle",
        "pauseDownload",
        "resumeDownload",
        "deleteDownload",
    ] {
        assert_eq!(mutation_field(field_name)["type"]["kind"], "NON_NULL");
        assert_eq!(
            mutation_field(field_name)["type"]["ofType"]["name"],
            "DownloadQueueActionPayload"
        );
    }

    let action_fields = body["data"]["actionPayload"]["fields"]
        .as_array()
        .expect("DownloadQueueActionPayload should expose fields");
    let action_field = |name: &str| {
        action_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("action payload field should exist")
    };

    assert_eq!(action_field("kind")["type"]["kind"], "NON_NULL");
    assert_eq!(
        action_field("kind")["type"]["ofType"]["name"],
        "DownloadQueueActionKindValue"
    );
    assert_eq!(
        action_field("downloadClientItemId")["type"]["kind"],
        "NON_NULL"
    );
    assert_eq!(action_field("removed")["type"]["kind"], "NON_NULL");
    assert_eq!(
        action_field("queueItem")["type"]["name"],
        "DownloadQueueItemPayload"
    );
    assert_eq!(action_field("importId")["type"]["name"], "String");

    let action_kind_names: Vec<&str> = body["data"]["actionKind"]["enumValues"]
        .as_array()
        .expect("DownloadQueueActionKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(action_kind_names.contains(&"queued_manual_import"));
    assert!(action_kind_names.contains(&"assigned_tracked_download_title"));
    assert!(action_kind_names.contains(&"delete_queued"));
}

#[tokio::test]
async fn graphql_queue_manual_import_returns_ok_and_persists_pending_request() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Queued Manual Import Movie", "movie").await;

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"listgroups""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"history""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "id": "scryer-rpc",
            "result": [{
                "NZBID": 999,
                "Name": "Queued.Manual.Import.2024.1080p.WEB-DL-GROUP",
                "NZBName": "Queued.Manual.Import.2024.1080p.WEB-DL-GROUP",
                "NZBFilename": "Queued.Manual.Import.2024.1080p.WEB-DL-GROUP.nzb",
                "DestDir": "/downloads/completed/movies/Queued.Manual.Import.2024.1080p.WEB-DL-GROUP",
                "FinalDir": "/downloads/completed/movies/Queued.Manual.Import.2024.1080p.WEB-DL-GROUP",
                "Category": "movies",
                "FileSizeLo": 0,
                "FileSizeHi": 1,
                "FileSizeMB": 4096,
                "DownloadedSizeLo": 0,
                "DownloadedSizeHi": 1,
                "DownloadedSizeMB": 4096,
                "DownloadTimeSec": 600,
                "PostTotalTimeSec": 120,
                "ParTimeSec": 30,
                "RepairTimeSec": 0,
                "UnpackTimeSec": 90,
                "Status": "SUCCESS/ALL",
                "TotalArticles": 50000,
                "SuccessArticles": 50000,
                "FailedArticles": 0,
                "Health": 1000,
                "CriticalHealth": 986,
                "MinPostTime": Utc::now().timestamp() - 600,
                "MaxPostTime": Utc::now().timestamp() - 300,
                "HistoryTime": Utc::now().timestamp(),
                "MarkStatus": "NONE",
                "Parameters": []
            }]
        })))
        .mount(&ctx.nzbget_server)
        .await;

    let client = ctx.http_client();
    let response = client
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"
                mutation QueueManualImport($input: QueueManualImportInput!) {
                  queueManualImport(input: $input) {
                    kind
                    importId
                    queueItem { id }
                  }
                }
            "#,
            "variables": {
                "input": {
                    "titleId": title_id,
                    "clientType": "nzbget",
                    "downloadClientItemId": "999"
                }
            }
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), 200);
    let body: Value = response
        .json()
        .await
        .expect("response should be valid json");
    assert_no_errors(&body);

    let import_id = body["data"]["queueManualImport"]["importId"]
        .as_str()
        .expect("queue manual import should return an import id");
    assert_eq!(
        body["data"]["queueManualImport"]["kind"],
        json!("queued_manual_import")
    );

    let history_body = gql(
        &ctx,
        r#"
        {
          importHistory {
            id
            importType
            status
            sourceRef
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&history_body);

    let queued = history_body["data"]["importHistory"]
        .as_array()
        .expect("import history should be an array")
        .iter()
        .find(|entry| entry["id"].as_str() == Some(import_id))
        .expect("queued manual import should be present in history");
    assert_eq!(queued["sourceRef"], json!("999"));
    assert_eq!(queued["importType"], json!("manual_import"));
    assert_eq!(queued["status"], json!("pending"));
}

#[tokio::test]
async fn graphql_delete_download_returns_ok_and_persists_queued_delete_command() {
    let ctx = TestContext::new().await;
    let client = ctx.http_client();
    let response = client
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"
                mutation DeleteDownload($input: DeleteDownloadInput!) {
                  deleteDownload(input: $input) {
                    kind
                    commandId
                    removed
                    clientType
                    queueItem { id }
                  }
                }
            "#,
            "variables": {
                "input": {
                    "clientType": "nzbget",
                    "downloadClientItemId": "queued-delete-download-1",
                    "isHistory": true
                }
            }
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), 200);
    let body: Value = response
        .json()
        .await
        .expect("response should be valid json");
    assert_no_errors(&body);

    let action = &body["data"]["deleteDownload"];
    let command_id = action["commandId"]
        .as_str()
        .expect("delete download should return a queued command id");
    assert_eq!(action["kind"], json!("delete_queued"));
    assert_eq!(action["removed"], json!(false));
    assert_eq!(action["clientType"], json!("nzbget"));
    assert!(action["queueItem"].is_null());

    let queued = sqlx::query(
        "SELECT action, client_type, download_client_item_id, is_history, status
         FROM download_queue_commands
         WHERE id = ?",
    )
    .bind(command_id)
    .fetch_one(ctx.db.pool())
    .await
    .expect("queued delete command should be persisted");

    assert_eq!(
        queued
            .try_get::<String, _>("action")
            .expect("action should be readable"),
        "delete"
    );
    assert_eq!(
        queued
            .try_get::<String, _>("client_type")
            .expect("client_type should be readable"),
        "nzbget"
    );
    assert_eq!(
        queued
            .try_get::<String, _>("download_client_item_id")
            .expect("download_client_item_id should be readable"),
        "queued-delete-download-1"
    );
    assert!(
        queued
            .try_get::<i64, _>("is_history")
            .expect("is_history should be readable")
            != 0
    );
    assert_eq!(
        queued
            .try_get::<String, _>("status")
            .expect("status should be readable"),
        "queued"
    );
}

#[tokio::test]
async fn graphql_delete_download_marks_history_item_completed_after_poller_runs() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let create_config_body = gql(
        &ctx,
        r#"
        mutation CreateDownloadClientConfig($input: CreateDownloadClientConfigInput!) {
          createDownloadClientConfig(input: $input) {
            id
            clientType
            isEnabled
          }
        }
        "#,
        json!({
            "input": {
                "name": "NZBGet",
                "clientType": "nzbget",
                "configJson": "{}",
                "isEnabled": true
            }
        }),
    )
    .await;
    assert_no_errors(&create_config_body);
    assert_eq!(
        create_config_body["data"]["createDownloadClientConfig"]["clientType"],
        json!("nzbget")
    );

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"listgroups""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"postqueue""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/postqueue.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"history""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "result": {
                "History": [{
                    "NZBID": 123,
                    "Name": "Queued Delete Download",
                    "Status": "SUCCESS",
                    "HistoryTime": Utc::now().timestamp(),
                    "FileSizeMB": 10
                }]
            },
            "id": "scryer-rpc"
        })))
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"editqueue""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "result": true,
            "id": "scryer-rpc"
        })))
        .mount(&ctx.nzbget_server)
        .await;

    let delete_body = gql(
        &ctx,
        r#"
        mutation DeleteDownload($input: DeleteDownloadInput!) {
          deleteDownload(input: $input) {
            kind
            commandId
            removed
          }
        }
        "#,
        json!({
            "input": {
                "clientType": "nzbget",
                "downloadClientItemId": "123",
                "isHistory": true
            }
        }),
    )
    .await;
    assert_no_errors(&delete_body);
    assert_eq!(
        delete_body["data"]["deleteDownload"]["kind"],
        json!("delete_queued")
    );

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        ctx.app.clone(),
        token.child_token(),
    ));

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let status: Option<String> = sqlx::query_scalar(
                "SELECT status
                 FROM download_queue_commands
                 WHERE client_type = 'nzbget'
                   AND download_client_item_id = '123'
                   AND is_history = 1",
            )
            .fetch_optional(ctx.db.pool())
            .await
            .expect("queued delete status should load");
            if status.as_deref() == Some("completed") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued delete should complete");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    let queue_body = gql(
        &ctx,
        r#"
        {
          downloadQueue(includeAllActivity: true) {
            downloadClientItemId
            state
            deleteStatus
            deleteErrorMessage
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&queue_body);

    assert!(
        queue_body["data"]["downloadQueue"]
            .as_array()
            .expect("download queue should be an array")
            .iter()
            .all(|item| item["downloadClientItemId"].as_str() != Some("123"))
    );

    let history_body = gql(
        &ctx,
        r#"
        {
                    downloadHistory(limit: 100, offset: 0, filters: [all]) {
            items {
              downloadClientItemId
              state
              deleteStatus
              deleteErrorMessage
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&history_body);

    let item = history_body["data"]["downloadHistory"]["items"]
        .as_array()
        .expect("download history should be an array")
        .iter()
        .find(|item| item["downloadClientItemId"].as_str() == Some("123"))
        .expect("history item should remain visible in history");

    assert_eq!(item["state"], json!("completed"));
    assert_eq!(item["deleteStatus"], json!("completed"));
    assert!(item["deleteErrorMessage"].is_null());
}

#[tokio::test]
async fn graphql_introspection_exposes_wanted_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          wantedItem: __type(name: "WantedItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          wantedStatus: __type(name: "WantedStatusValue") {
            enumValues { name }
          }
          wantedMediaType: __type(name: "WantedMediaTypeValue") {
            enumValues { name }
          }
          wantedSearchPhase: __type(name: "WantedSearchPhaseValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["wantedItem"]["fields"]
        .as_array()
        .expect("WantedItemPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("mediaType")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("mediaType")["type"]["ofType"]["name"],
        "WantedMediaTypeValue"
    );
    assert_eq!(field("searchPhase")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("searchPhase")["type"]["ofType"]["name"],
        "WantedSearchPhaseValue"
    );
    assert_eq!(field("status")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("status")["type"]["ofType"]["name"],
        "WantedStatusValue"
    );

    let status_names: Vec<&str> = body["data"]["wantedStatus"]["enumValues"]
        .as_array()
        .expect("WantedStatusValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        status_names,
        vec!["wanted", "grabbed", "paused", "completed"]
    );

    let media_type_names: Vec<&str> = body["data"]["wantedMediaType"]["enumValues"]
        .as_array()
        .expect("WantedMediaTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        media_type_names,
        vec!["movie", "episode", "interstitial_movie"]
    );

    let search_phase_names: Vec<&str> = body["data"]["wantedSearchPhase"]["enumValues"]
        .as_array()
        .expect("WantedSearchPhaseValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        search_phase_names,
        vec![
            "pre_air",
            "pre_release",
            "primary",
            "secondary",
            "long_tail"
        ]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_import_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          importRecord: __type(name: "ImportRecordPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          importResult: __type(name: "ImportResultPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          importStatus: __type(name: "ImportStatusValue") {
            enumValues { name }
          }
          importType: __type(name: "ImportTypeValue") {
            enumValues { name }
          }
          importDecision: __type(name: "ImportDecisionValue") {
            enumValues { name }
          }
          importSkipReason: __type(name: "ImportSkipReasonValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let record_fields = body["data"]["importRecord"]["fields"]
        .as_array()
        .expect("ImportRecordPayload should expose fields");
    let record_field = |name: &str| {
        record_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(record_field("importType")["type"]["kind"], "NON_NULL");
    assert_eq!(
        record_field("importType")["type"]["ofType"]["name"],
        "ImportTypeValue"
    );
    assert_eq!(record_field("status")["type"]["kind"], "NON_NULL");
    assert_eq!(
        record_field("status")["type"]["ofType"]["name"],
        "ImportStatusValue"
    );
    assert_eq!(
        record_field("decision")["type"]["name"],
        "ImportDecisionValue"
    );
    assert_eq!(
        record_field("skipReason")["type"]["name"],
        "ImportSkipReasonValue"
    );

    let result_fields = body["data"]["importResult"]["fields"]
        .as_array()
        .expect("ImportResultPayload should expose fields");
    let result_field = |name: &str| {
        result_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(result_field("decision")["type"]["kind"], "NON_NULL");
    assert_eq!(
        result_field("decision")["type"]["ofType"]["name"],
        "ImportDecisionValue"
    );
    assert_eq!(
        result_field("skipReason")["type"]["name"],
        "ImportSkipReasonValue"
    );

    let import_status_names: Vec<&str> = body["data"]["importStatus"]["enumValues"]
        .as_array()
        .expect("ImportStatusValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        import_status_names,
        vec![
            "pending",
            "running",
            "processing",
            "completed",
            "failed",
            "skipped"
        ]
    );

    let import_type_names: Vec<&str> = body["data"]["importType"]["enumValues"]
        .as_array()
        .expect("ImportTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_type_names.contains(&"series_download"));
    assert!(import_type_names.contains(&"rename_io_failed"));

    let import_decision_names: Vec<&str> = body["data"]["importDecision"]["enumValues"]
        .as_array()
        .expect("ImportDecisionValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        import_decision_names,
        vec![
            "imported",
            "rejected",
            "skipped",
            "conflict",
            "unmatched",
            "failed"
        ]
    );

    let import_skip_reason_names: Vec<&str> = body["data"]["importSkipReason"]["enumValues"]
        .as_array()
        .expect("ImportSkipReasonValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_skip_reason_names.contains(&"password_required"));
    assert!(import_skip_reason_names.contains(&"post_download_rule_blocked"));
}

#[tokio::test]
async fn graphql_introspection_exposes_activity_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          activityEvent: __type(name: "ActivityEventPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          activityKind: __type(name: "ActivityKindValue") {
            enumValues { name }
          }
          activitySeverity: __type(name: "ActivitySeverityValue") {
            enumValues { name }
          }
          activityChannel: __type(name: "ActivityChannelValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["activityEvent"]["fields"]
        .as_array()
        .expect("ActivityEventPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("kind")["type"]["kind"], "NON_NULL");
    assert_eq!(field("kind")["type"]["ofType"]["name"], "ActivityKindValue");
    assert_eq!(field("severity")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("severity")["type"]["ofType"]["name"],
        "ActivitySeverityValue"
    );
    assert_eq!(field("channels")["type"]["kind"], "NON_NULL");
    assert_eq!(field("channels")["type"]["ofType"]["kind"], "LIST");
    assert_eq!(
        field("channels")["type"]["ofType"]["ofType"]["kind"],
        "NON_NULL"
    );
    assert_eq!(
        field("channels")["type"]["ofType"]["ofType"]["ofType"]["name"],
        "ActivityChannelValue"
    );

    let activity_kind_names: Vec<&str> = body["data"]["activityKind"]["enumValues"]
        .as_array()
        .expect("ActivityKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(activity_kind_names.contains(&"title_updated"));
    assert!(activity_kind_names.contains(&"metadata_hydration_completed"));
    assert!(activity_kind_names.contains(&"import_rejected"));

    let activity_severity_names: Vec<&str> = body["data"]["activitySeverity"]["enumValues"]
        .as_array()
        .expect("ActivitySeverityValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        activity_severity_names,
        vec!["info", "success", "warning", "error"]
    );

    let activity_channel_names: Vec<&str> = body["data"]["activityChannel"]["enumValues"]
        .as_array()
        .expect("ActivityChannelValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(activity_channel_names, vec!["web_ui", "toast"]);
}

// ---------------------------------------------------------------------------
// Title CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_list_titles_starts_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn graphql_add_title_movie() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Movie", "movie").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_tv() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Series", "series").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_anime() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Anime", "anime").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_with_structured_options() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    id
                    qualityProfileId
                    rootFolderPath
                    monitorType
                    useSeasonFolders
                    monitorSpecials
                    interSeasonMovies
                    fillerPolicy
                    recapPolicy
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Configured Anime",
                "facet": "anime",
                "monitored": true,
                "tags": ["favorite"],
                "options": {
                    "qualityProfileId": "anime-hd",
                    "rootFolderPath": "/library/anime",
                    "monitorType": "futureEpisodes",
                    "useSeasonFolders": false,
                    "monitorSpecials": true,
                    "interSeasonMovies": false,
                    "fillerPolicy": "skip_filler",
                    "recapPolicy": "skip_recap"
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let title = &body["data"]["addTitle"]["title"];
    assert_eq!(title["qualityProfileId"], "anime-hd");
    assert_eq!(title["rootFolderPath"], "/library/anime");
    assert_eq!(title["monitorType"], "futureEpisodes");
    assert_eq!(title["useSeasonFolders"], false);
    assert_eq!(title["monitorSpecials"], true);
    assert_eq!(title["interSeasonMovies"], false);
    assert_eq!(title["fillerPolicy"], "skip_filler");
    assert_eq!(title["recapPolicy"], "skip_recap");
}

#[tokio::test]
async fn graphql_add_title_returns_async_hydration_payload_fields() {
    let ctx = TestContext::new().await;
    let query = r#"mutation($input: AddTitleInput!) {
        addTitle(input: $input) {
            metadataHydrationState
            reusedExistingTitle
            reusedQueuedDownload
            title {
                id
                name
            }
        }
    }"#;
    let variables = json!({
        "input": {
            "name": "Async Payload Movie",
            "facet": "movie",
            "monitored": true,
            "tags": [],
            "externalIds": [{ "source": "tvdb", "value": "123456" }]
        }
    });

    let first = gql(&ctx, query, variables.clone()).await;
    assert_no_errors(&first);
    assert_eq!(
        first["data"]["addTitle"]["metadataHydrationState"],
        "pending"
    );
    assert_eq!(first["data"]["addTitle"]["reusedExistingTitle"], false);
    assert_eq!(first["data"]["addTitle"]["reusedQueuedDownload"], false);

    let second = gql(&ctx, query, variables).await;
    assert_no_errors(&second);
    assert_eq!(
        second["data"]["addTitle"]["metadataHydrationState"],
        "pending"
    );
    assert_eq!(second["data"]["addTitle"]["reusedExistingTitle"], true);
    assert_eq!(second["data"]["addTitle"]["reusedQueuedDownload"], false);
    assert_eq!(
        second["data"]["addTitle"]["title"]["id"],
        first["data"]["addTitle"]["title"]["id"]
    );
}

#[tokio::test]
async fn graphql_add_title_then_list() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Listed Movie", "movie").await;

    let body = gql(&ctx, "{ titles { id name facet } }", json!({})).await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["id"], title_id);
    assert!(
        titles[0]["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
    );
    assert_eq!(titles[0]["facet"], "movie");
}

#[tokio::test]
async fn graphql_add_multiple_titles() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie One", "movie").await;
    add_test_title(&ctx, "Series One", "series").await;
    add_test_title(&ctx, "Anime One", "anime").await;

    let body = gql(&ctx, "{ titles { id facet } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_titles_by_external_ids_returns_catalog_titles() {
    let ctx = TestContext::new().await;
    let first = create_catalog_title(
        &ctx,
        "Mario",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "18861".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let duplicate = create_catalog_title(
        &ctx,
        "Mario Duplicate",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "18861".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let second = create_catalog_title(
        &ctx,
        "The Super Mario Galaxy Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "354713".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"query($source: String!, $values: [String!]!) {
          titlesByExternalIds(source: $source, values: $values) {
            id
            name
            facet
            externalIds { source value }
          }
        }"#,
        json!({
            "source": "tvdb",
            "values": ["18861", "18861", "000000", "354713"]
        }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titlesByExternalIds"]
        .as_array()
        .expect("titles array");
    let expected_first_match = if first.id <= duplicate.id {
        first.id.as_str()
    } else {
        duplicate.id.as_str()
    };
    assert_eq!(titles.len(), 2);
    assert_eq!(titles[0]["id"].as_str(), Some(expected_first_match));
    assert_eq!(titles[1]["id"].as_str(), Some(second.id.as_str()));
}

#[tokio::test]
async fn graphql_titles_are_sorted_by_display_name() {
    let ctx = TestContext::new().await;
    create_catalog_title(&ctx, "zeta movie", MediaFacet::Movie, vec![], vec![], true).await;
    create_catalog_title(&ctx, "Alpha Movie", MediaFacet::Movie, vec![], vec![], true).await;
    create_catalog_title(&ctx, "beta movie", MediaFacet::Movie, vec![], vec![], true).await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { name } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().unwrap();
    let names: Vec<&str> = titles
        .iter()
        .map(|title| title["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Alpha Movie", "beta movie", "zeta movie"]);
}

#[tokio::test]
async fn graphql_titles_expose_episode_progress_excluding_specials() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Episode Progress Show",
        MediaFacet::Series,
        vec![],
        vec![],
        false,
    )
    .await;

    let season_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let specials_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");

    let season_zero_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Season 0".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season zero collection");

    let regular_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "1", "S01E01").await;
    let mut regular_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "2", "S01E02").await;
    let regular_episode_3 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "3", "S01E03").await;
    let special_episode_1 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "1", "S00E01").await;
    let _special_episode_2 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "2", "S00E02").await;
    let season_zero_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "3", "S00E03").await;
    let _season_zero_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "4", "S00E04").await;

    regular_episode_2 = ctx
        .catalog
        .update_episode(
            &regular_episode_2.id,
            EpisodeUpdate {
                air_date: Some("2024-01-08".to_string()),
                monitored: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update episode monitored flag");

    let regular_episode_1 = ctx
        .catalog
        .update_episode(
            &regular_episode_1.id,
            EpisodeUpdate {
                air_date: Some("2024-01-01".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update first regular episode air date");

    ctx
        .catalog
        .update_episode(
            &regular_episode_3.id,
            EpisodeUpdate {
                air_date: Some("2024-01-15".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update third regular episode air date");

    for (index, episode) in [
        regular_episode_1,
        regular_episode_2,
        special_episode_1,
        season_zero_episode_1,
    ]
    .into_iter()
    .enumerate()
    {
        let file_path = media_root
            .path()
            .join(format!("Episode.Progress.Show.file-{index}.mkv"));
        let file_id = ctx
            .library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 4_096 + index as i64,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.db
            .link_file_to_episode(&file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { id name episodesOwned episodesMonitored episodesTotal } }"#,
        json!({ "facet": "series" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("series title should be listed");

    assert_eq!(listed_title["name"], "Episode Progress Show");
    assert_eq!(listed_title["episodesOwned"], 2);
    assert_eq!(listed_title["episodesMonitored"], 2);
    assert_eq!(listed_title["episodesTotal"], 3);
}

#[tokio::test]
async fn graphql_titles_exclude_tba_or_incomplete_metadata_episodes_from_progress_counts() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Progress Count Filter Show",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("4".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let countable_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create countable episode");

    let tba_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("TBA".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create tba episode");

    let untitled_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: None,
            air_date: Some("2024-01-15".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create untitled episode");

    let undated_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("4".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E04".to_string()),
            title: Some("Named but undated".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create undated episode");

    for (index, episode) in [
        countable_episode.clone(),
        tba_episode,
        untitled_episode,
        undated_episode,
    ]
    .into_iter()
    .enumerate()
    {
        let file_path = media_root
            .path()
            .join(format!("Progress.Count.Filter.Show.file-{index}.mkv"));
        let file_id = ctx
            .library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 8_192 + index as i64,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.db
            .link_file_to_episode(&file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { id name episodesOwned episodesMonitored episodesTotal } }"#,
        json!({ "facet": "series" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("series title should be listed");

    assert_eq!(listed_title["name"], "Progress Count Filter Show");
    assert_eq!(listed_title["episodesOwned"], 1);
    assert_eq!(listed_title["episodesMonitored"], 1);
    assert_eq!(listed_title["episodesTotal"], 1);
}

#[tokio::test]
async fn graphql_titles_expose_matched_size_bytes_only_for_anime_titles() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Matched Size Anime",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let specials_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");

    let season_zero_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Season 0".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season zero collection");

    let interstitial_path = media_root
        .path()
        .join("Matched.Size.Anime.Interstitial.1080p.mkv");
    let _interstitial_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Interstitial".to_string()),
            ordered_path: Some(interstitial_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let regular_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "1", "S01E01").await;
    let regular_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "2", "S01E02").await;
    let special_episode =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "1", "S00E01").await;
    let season_zero_episode =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "2", "S00E02").await;

    let multi_episode_path = media_root.path().join("Matched.Size.Anime.S01E01-E02.mkv");
    let multi_episode_file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: multi_episode_path.to_string_lossy().to_string(),
            size_bytes: 1_000,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert multi-episode file");
    for episode_id in [&regular_episode_1.id, &regular_episode_2.id] {
        ctx.db
            .link_file_to_episode(&multi_episode_file_id, episode_id)
            .await
            .expect("link multi-episode file");
    }

    let special_path = media_root.path().join("Matched.Size.Anime.Special.mkv");
    let special_file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: special_path.to_string_lossy().to_string(),
            size_bytes: 200,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert special file");
    ctx.db
        .link_file_to_episode(&special_file_id, &special_episode.id)
        .await
        .expect("link special file");

    let season_zero_path = media_root.path().join("Matched.Size.Anime.Season.Zero.mkv");
    let season_zero_file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: season_zero_path.to_string_lossy().to_string(),
            size_bytes: 300,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert season zero file");
    ctx.db
        .link_file_to_episode(&season_zero_file_id, &season_zero_episode.id)
        .await
        .expect("link season zero file");

    ctx.library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: interstitial_path.to_string_lossy().to_string(),
            size_bytes: 400,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial file");

    ctx.library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: media_root
                .path()
                .join("Matched.Size.Anime.Unmatched.Extra.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 500,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert unmatched file");

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { id name sizeBytes } }"#,
        json!({ "facet": "anime" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("anime title should be listed");

    assert_eq!(listed_title["name"], "Matched Size Anime");
    assert_eq!(listed_title["sizeBytes"], json!(1_900));
}

#[tokio::test]
async fn graphql_titles_expose_matched_size_bytes_only_for_movies() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Matched Size Movie",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;

    let matched_path = media_root.path().join("Matched.Size.Movie.2160p.mkv");
    ctx.catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("Matched Size Movie".to_string()),
            ordered_path: Some(matched_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    ctx.library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: matched_path.to_string_lossy().to_string(),
            size_bytes: 1_200,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert matched movie file");

    ctx.library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: media_root
                .path()
                .join("Matched.Size.Movie.Unmatched.Extra.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 700,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert unmatched movie file");

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { id name sizeBytes } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("movie title should be listed");

    assert_eq!(listed_title["name"], "Matched Size Movie");
    assert_eq!(listed_title["sizeBytes"], json!(1_200));
}

#[tokio::test]
async fn graphql_get_title_by_id() {
    let ctx = TestContext::new().await;
    let expected_name = "Specific Movie";
    let id = add_test_title(&ctx, expected_name, "movie").await;

    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["title"]["name"], expected_name);
    assert_eq!(body["data"]["title"]["monitored"], true);
}

#[tokio::test]
async fn graphql_get_title_not_found() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name } }"#,
        json!({ "id": "nonexistent-id" }),
    )
    .await;
    assert!(
        body["data"]["title"].is_null(),
        "should return null for nonexistent title"
    );
}

#[tokio::test]
async fn graphql_set_title_monitored() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Monitor Test", "movie").await;

    // Disable monitoring
    let body = gql(
        &ctx,
        r#"mutation($input: SetTitleMonitoredInput!) {
            setTitleMonitored(input: $input) { id monitored }
        }"#,
        json!({ "input": { "titleId": id, "monitored": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["setTitleMonitored"]["monitored"], false);

    // Verify via query
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_eq!(body["data"]["title"]["monitored"], false);
}

#[tokio::test]
async fn graphql_update_collection_monitored_matches_set_collection_monitored_side_effects() {
    let ctx = TestContext::new().await;
    let (set_title, set_collection, set_episode) =
        create_series_monitoring_fixture(&ctx, "Set Collection Flow", "61001").await;
    let (update_title, update_collection, update_episode) =
        create_series_monitoring_fixture(&ctx, "Update Collection Flow", "61002").await;

    let set_enable = gql(
        &ctx,
        r#"mutation($input: SetCollectionMonitoredInput!) {
            setCollectionMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": set_collection.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&set_enable);

    let update_enable = gql(
        &ctx,
        r#"mutation($input: UpdateCollectionInput!) {
            updateCollection(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": update_collection.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&update_enable);

    let set_enabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_enabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_enabled_summary, update_enabled_summary);
    assert_eq!(
        set_enabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: true,
            wanted_count: 1,
        }
    );

    let set_disable = gql(
        &ctx,
        r#"mutation($input: SetCollectionMonitoredInput!) {
            setCollectionMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": set_collection.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&set_disable);

    let update_disable = gql(
        &ctx,
        r#"mutation($input: UpdateCollectionInput!) {
            updateCollection(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": update_collection.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&update_disable);

    let set_disabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_disabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_disabled_summary, update_disabled_summary);
    assert_eq!(
        set_disabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: false,
            episode_monitored: false,
            wanted_count: 0,
        }
    );
}

#[tokio::test]
async fn graphql_update_episode_monitored_matches_set_episode_monitored_side_effects() {
    let ctx = TestContext::new().await;
    let (set_title, set_collection, set_episode) =
        create_series_monitoring_fixture(&ctx, "Set Episode Flow", "62001").await;
    let (update_title, update_collection, update_episode) =
        create_series_monitoring_fixture(&ctx, "Update Episode Flow", "62002").await;

    let set_enable = gql(
        &ctx,
        r#"mutation($input: SetEpisodeMonitoredInput!) {
            setEpisodeMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": set_episode.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&set_enable);

    let update_enable = gql(
        &ctx,
        r#"mutation($input: UpdateEpisodeInput!) {
            updateEpisode(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": update_episode.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&update_enable);

    let set_enabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_enabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_enabled_summary, update_enabled_summary);
    assert_eq!(
        set_enabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: true,
            wanted_count: 1,
        }
    );

    let set_disable = gql(
        &ctx,
        r#"mutation($input: SetEpisodeMonitoredInput!) {
            setEpisodeMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": set_episode.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&set_disable);

    let update_disable = gql(
        &ctx,
        r#"mutation($input: UpdateEpisodeInput!) {
            updateEpisode(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": update_episode.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&update_disable);

    let set_disabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_disabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_disabled_summary, update_disabled_summary);
    assert_eq!(
        set_disabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: false,
            wanted_count: 0,
        }
    );
}

#[tokio::test]
async fn graphql_update_title_structured_options_merge_with_existing_tags() {
    let ctx = TestContext::new().await;
    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id }
            }
        }"#,
        json!({
            "input": {
                "name": "Option Update Anime",
                "facet": "anime",
                "monitored": true,
                "tags": ["favorite"]
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title_id = add_body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                id
                tags
                qualityProfileId
                rootFolderPath
                useSeasonFolders
                fillerPolicy
                recapPolicy
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "qualityProfileId": "anime-4k",
                    "rootFolderPath": "/custom/anime",
                    "useSeasonFolders": false,
                    "fillerPolicy": "skip_filler",
                    "recapPolicy": ""
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let updated = &body["data"]["updateTitle"];
    assert_eq!(updated["qualityProfileId"], "anime-4k");
    assert_eq!(updated["rootFolderPath"], "/custom/anime");
    assert_eq!(updated["useSeasonFolders"], false);
    assert_eq!(updated["fillerPolicy"], "skip_filler");
    assert!(updated["recapPolicy"].is_null());

    let tags = updated["tags"].as_array().expect("tags array");
    let tag_values: Vec<&str> = tags.iter().filter_map(|tag| tag.as_str()).collect();
    assert!(tag_values.contains(&"favorite"));
    assert!(tag_values.contains(&"scryer:quality-profile:anime-4k"));
    assert!(tag_values.contains(&"scryer:root-folder:/custom/anime"));
    assert!(tag_values.contains(&"scryer:season-folder:disabled"));
    assert!(tag_values.contains(&"scryer:filler-policy:skip_filler"));
    assert!(
        !tag_values
            .iter()
            .any(|tag| tag.starts_with("scryer:recap-policy:"))
    );
}

#[tokio::test]
async fn graphql_trigger_title_wanted_search() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Search Monitored Test", "movie").await;

    let body = gql(
        &ctx,
        r#"mutation($input: TriggerTitleWantedSearchInput!) {
            triggerTitleWantedSearch(input: $input) {
                queuedCount
                skippedInProgressCount
            }
        }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["triggerTitleWantedSearch"]["queuedCount"], 1);
    assert_eq!(
        body["data"]["triggerTitleWantedSearch"]["skippedInProgressCount"],
        0
    );

    let body = gql(
        &ctx,
        r#"query($titleId: String) {
            wantedItems(titleId: $titleId) {
                total
                items { titleId mediaType status }
            }
        }"#,
        json!({ "titleId": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["wantedItems"]["total"], 1);
    assert_eq!(body["data"]["wantedItems"]["items"][0]["titleId"], id);
    assert_eq!(
        body["data"]["wantedItems"]["items"][0]["mediaType"],
        "movie"
    );
    assert_eq!(body["data"]["wantedItems"]["items"][0]["status"], "wanted");
}

#[tokio::test]
async fn graphql_trigger_title_wanted_search_series_queues_all_monitored_episodes() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Search Monitored Series", vec![]).await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let body = gql(
        &ctx,
        r#"mutation($input: TriggerTitleWantedSearchInput!) {
            triggerTitleWantedSearch(input: $input) {
                queuedCount
                skippedInProgressCount
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["triggerTitleWantedSearch"]["queuedCount"], 2);
    assert_eq!(
        body["data"]["triggerTitleWantedSearch"]["skippedInProgressCount"],
        0
    );

    let body = gql(
        &ctx,
        r#"query($titleId: String) {
            wantedItems(titleId: $titleId) {
                total
                items { titleId mediaType status }
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["wantedItems"]["total"], 2);
    let items = body["data"]["wantedItems"]["items"]
        .as_array()
        .expect("wanted items array");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| item["titleId"] == title.id));
    assert!(items.iter().all(|item| item["mediaType"] == "episode"));
    assert!(items.iter().all(|item| item["status"] == "wanted"));
}

#[tokio::test]
async fn graphql_scan_title_library() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Scan.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                    scanStatus
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(files[0]["scanStatus"], "scan_failed");

    let persisted_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = media_root.path().join(&title.name);
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.to_string_lossy().as_ref())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .all(|tag| tag != "scryer:season-folder:disabled")
    );

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(activity_kinds.iter().any(|kind| kind == "title_updated"));
}

#[tokio::test]
async fn graphql_scan_title_library_removes_stale_media_file_when_file_deleted_on_disk() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Stale Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Stale.Scan.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);

    std::fs::remove_file(&file_path).expect("remove scanned file from disk");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert!(
        files.is_empty(),
        "title scan should delete stale media_files rows when the file no longer exists on disk"
    );
}

#[tokio::test]
async fn graphql_scan_title_library_matches_x_episode_numbering_with_title_context() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Scan Show - 01x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_keeps_standard_episode_titles_with_special_in_name() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, _season_one_collection) =
        create_series_scan_title(&ctx, media_root.path(), "Attack on Titan", vec![]).await;

    let season_four = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "4".to_string(),
            label: Some("Season 4".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("29".to_string()),
            last_episode_number: Some("30".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season four collection");
    let episode_29 = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_four.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("29".to_string()),
            season_number: Some("4".to_string()),
            episode_label: Some("S04E29".to_string()),
            title: Some("The Final Chapters Special 1".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode 29");
    let episode_30 = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_four.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("30".to_string()),
            season_number: Some("4".to_string()),
            episode_label: Some("S04E30".to_string()),
            title: Some("The Final Chapters Special 2".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode 30");

    let season_dir = media_root.path().join(&title.name).join("Season 04");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path_29 =
        season_dir.join("Attack.on.Titan.S04E29.The.Final.Chapters.Special.1.1080p.WEB-DL.mkv");
    std::fs::write(&file_path_29, b"not-a-real-video").expect("write episode 29");
    let file_path_30 =
        season_dir.join("Attack.on.Titan.S04E30.The.Final.Chapters.Special.2.1080p.WEB-DL.mkv");
    std::fs::write(&file_path_30, b"not-a-real-video").expect("write episode 30");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| {
        file["episodeId"] == episode_29.id
            && file["filePath"] == file_path_29.to_string_lossy().to_string()
    }));
    assert!(files.iter().any(|file| {
        file["episodeId"] == episode_30.id
            && file["filePath"] == file_path_30.to_string_lossy().to_string()
    }));
}

#[tokio::test]
async fn graphql_scan_title_library_matches_numbered_special_episode_on_disk() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, _season_one_collection) =
        create_series_scan_title(&ctx, media_root.path(), "Special Scan Show", vec![]).await;

    let specials_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");
    let special_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(specials_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Special,
            episode_number: Some("1".to_string()),
            season_number: Some("0".to_string()),
            episode_label: Some("S00E01".to_string()),
            title: Some("OVA 1".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create special episode");

    let specials_dir = media_root.path().join(&title.name).join("Specials");
    std::fs::create_dir_all(&specials_dir).expect("create specials dir");
    let file_path = specials_dir.join("Special Scan Show - 01 - OVA 1080p WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write special episode");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], special_episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_matches_daily_episodes_by_air_date() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Daily Show", vec![]).await;
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Daily Episode".to_string()),
        air_date: Some("2024-03-15".to_string()),
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .catalog
        .create_episode(episode)
        .await
        .expect("create episode");

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Daily.Show.2024.03.15.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_disables_season_folders_for_flat_layout() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Flat Show", vec![]).await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let title_dir = media_root.path().join(&title.name);
    std::fs::create_dir_all(&title_dir).expect("create title dir");
    std::fs::write(
        title_dir.join("Flat.Show.S01E01.1080p.WEB-DL.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake video");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
}

#[tokio::test]
async fn graphql_scan_title_library_preserves_existing_layout_when_ambiguous() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) = create_series_scan_title(
        &ctx,
        media_root.path(),
        "Mixed Show",
        vec!["scryer:season-folder:disabled".to_string()],
    )
    .await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let title_dir = media_root.path().join(&title.name);
    let season_dir = title_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(title_dir.join("Mixed.Show.S01E01.1080p.WEB-DL.mkv"), b"one")
        .expect("write flat file");
    std::fs::write(
        season_dir.join("Mixed.Show.S01E02.1080p.WEB-DL.mkv"),
        b"two",
    )
    .expect("write season file");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
    assert_eq!(
        persisted_title
            .tags
            .iter()
            .filter(|tag| tag.starts_with("scryer:season-folder:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn library_series_scan_hydrates_without_creating_wanted_for_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "genres": ["Drama", "Thriller"],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Test Show Name");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Test Show Name</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .catalog
            .list(Some(MediaFacet::Series), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title = hydrated_title.expect("title should hydrate");
    assert!(!hydrated_title.monitored);

    let (wanted_items, total) = ctx
        .app
        .list_wanted_items(scryer_application::WantedItemsQuery {
            status: None,
            media_type: None,
            title_id: Some(hydrated_title.id.clone()),
            title_search: None,
            latest_decision_code: None,
            limit: 10,
            offset: 0,
        })
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_anime_scan_hydrates_and_relinks_files_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": 456789,
                    "name": "Hydrated Anime Title",
                    "sort_name": "Hydrated Anime Title",
                    "slug": "hydrated-anime-title",
                    "status": "Ended",
                    "year": 2021,
                    "first_aired": "2021-01-10",
                    "overview": "An anime hydration fixture.",
                    "network": "Tokyo MX",
                    "runtime_minutes": 24,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/456789/posters/test.jpg",
                    "country": "jpn",
                    "genres": ["Animation"],
                    "aliases": ["Hydrated Anime Alias"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1001001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2001001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Episode 1",
                            "aired": "2021-01-10",
                            "runtime_minutes": 24,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "Episode 1 overview.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Anime Scan [SubsPlease]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Anime Scan</title><tvdbid>456789</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Anime.Scan.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let mut hydrated_title = None;
    let mut linked_files = Vec::new();
    for _ in 0..100 {
        let titles = ctx
            .catalog
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles");
        assert_eq!(titles.len(), 1);
        let files = ctx
            .library_state
            .list_media_files_for_title(&titles[0].id)
            .await
            .expect("list media files");
        if titles[0].metadata_fetched_at.is_some()
            && files.iter().any(|file| file.episode_id.is_some())
        {
            hydrated_title = Some(titles[0].clone());
            linked_files = files;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title = hydrated_title.expect("anime title should hydrate and relink files");
    assert_eq!(hydrated_title.name, "Hydrated Anime Title");
    assert!(hydrated_title.metadata_fetched_at.is_some());
    assert_eq!(
        hydrated_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert!(
        linked_files[0].episode_id.is_some(),
        "linked file should target a hydrated episode"
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_anime_scan_prefers_tvshow_nfo_identity_for_bastard_fixture() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": 415677,
                    "name": "Bastard!! Correct Match",
                    "sort_name": "Bastard!! Correct Match",
                    "slug": "bastard-correct-match",
                    "status": "Ended",
                    "year": 2022,
                    "first_aired": "2022-06-30",
                    "overview": "A regression fixture for the Bastard!! anime scan path.",
                    "network": "Netflix",
                    "runtime_minutes": 24,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/415677/posters/test.jpg",
                    "country": "jpn",
                    "genres": ["Animation", "Fantasy"],
                    "aliases": ["Bastard!! Ankoku no Hakaishin"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 14156771,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 24156771,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Episode 1",
                            "aired": "2022-06-30",
                            "runtime_minutes": 24,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "Episode 1 overview.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let bastard_tvshow_nfo = r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<tvshow>
  <plot>The characters in BASTARD!! have forsaken technology and have traded it in for magic and the occult arts. The story revolves around a group of five magicians who set out to rebuild their world after a rampaging God destroys it and tips it into chaos. Driven by their leader Dark Schneider, Gara, Arshes Nei, Abigail and Kall Su set out to conquer kingdoms in the hope of rebuilding them into utopias, with each of the magicians having their own personal agenda in doing so. However, Dark Schneider's often reckless manner and short temper leads him to wreak havoc where ever he goes. He is trapped by a group of clerics in a baby boy, leaving the four without a leader.Fifteen years later, the four still continue their quest but their motives have changed to despotic world domination, and it is up to Dark Schneider to show them the "error of their ways", when he is freed by the clerics who once imprisoned him, so that he may defend them against his former comrades. It has now become Dark Schneider's responsibility to track down each one of his companions and set them on the right path again, but what seems to be the once reckless and amoral Dark Schneider has changed over his fifteen year imprisonment within the boy, Lucien Renlen.</plot>
  <outline>The characters in BASTARD!! have forsaken technology and have traded it in for magic and the occult arts. The story revolves around a group of five magicians who set out to rebuild their world after a rampaging God destroys it and tips it into chaos. Driven by their leader Dark Schneider, Gara, Arshes Nei, Abigail and Kall Su set out to conquer kingdoms in the hope of rebuilding them into utopias, with each of the magicians having their own personal agenda in doing so. However, Dark Schneider's often reckless manner and short temper leads him to wreak havoc where ever he goes. He is trapped by a group of clerics in a baby boy, leaving the four without a leader.Fifteen years later, the four still continue their quest but their motives have changed to despotic world domination, and it is up to Dark Schneider to show them the "error of their ways", when he is freed by the clerics who once imprisoned him, so that he may defend them against his former comrades. It has now become Dark Schneider's responsibility to track down each one of his companions and set them on the right path again, but what seems to be the once reckless and amoral Dark Schneider has changed over his fifteen year imprisonment within the boy, Lucien Renlen.</outline>
  <lockdata>false</lockdata>
  <dateadded>2026-04-21 04:22:41</dateadded>
  <title>Bastard!!</title>
  <originaltitle>Bastard!! Ankoku no Hakaishin</originaltitle>
  <trailer>plugin://plugin.video.youtube/play/?video_id=_Iqc-dG8peA</trailer>
  <trailer>plugin://plugin.video.youtube/play/?video_id=Vt4zSf3CfRA</trailer>
  <rating>5</rating>
  <year>2022</year>
  <mpaa>TV-MA</mpaa>
  <collectionnumber>156898</collectionnumber>
  <imdb_id>tt17736234</imdb_id>
  <tmdbid>156898</tmdbid>
  <premiered>1992-08-25</premiered>
  <releasedate>1992-08-25</releasedate>
  <enddate>1993-06-25</enddate>
  <runtime>25</runtime>
  <genre>Anime</genre>
  <genre>magic</genre>
  <genre>stereotypes</genre>
  <genre>super power</genre>
  <genre>violence</genre>
  <studio />
  <studio>Netflix</studio>
  <tag>anime</tag>
  <tag>based on manga</tag>
  <tag>combat</tag>
  <tag>dark fantasy</tag>
  <tag>ecchi</tag>
  <tag>heavy metal</tag>
  <tag>magic</tag>
  <tag>original net animation (ona)</tag>
  <tag>remake</tag>
  <tag>seinen</tag>
  <anidbid>10</anidbid>
  <tvdbid>415677</tvdbid>
  <tvdbslugid>bastard-2022</tvdbslugid>
  <art>
    <poster>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/poster.jpg</poster>
    <fanart>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/backdrop.jpg</fanart>
  </art>
  <id>415677</id>
  <episodeguide>
    <url cache="415677.xml">http://www.thetvdb.com/api/1D62F2F90030C444/series/415677/all/en.zip</url>
  </episodeguide>
  <season>-1</season>
  <episode>-1</episode>
  <status>Ended</status>
</tvshow>"#;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Bastard!! (2022)");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(show_dir.join("tvshow.nfo"), bastard_tvshow_nfo).expect("write tvshow.nfo");
    std::fs::write(
        season_dir.join("Bastard!! (2022) - S01E01 (1) - 1080p.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake video");
    std::fs::write(
        season_dir.join("Bastard!! (2022) - S01E01 (1) - 1080p.nfo"),
        b"<episodedetails><title>Episode 1</title></episodedetails>",
    )
    .expect("write episode nfo");
    std::fs::write(season_dir.join("season.nfo"), b"<season></season>").expect("write season nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.unmatched, 0);

    let mut hydrated_title = None;
    for _ in 0..100 {
        let titles = ctx
            .catalog
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles");
        assert_eq!(titles.len(), 1);
        let title = &titles[0];
        if title.metadata_fetched_at.is_some() {
            hydrated_title = Some(title.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title =
        hydrated_title.expect("anime title should hydrate from tvshow.nfo identity");
    assert_eq!(hydrated_title.name, "Bastard!! Correct Match");
    assert!(hydrated_title.metadata_fetched_at.is_some());
    assert_eq!(
        hydrated_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        hydrated_title
            .external_ids
            .iter()
            .any(|id| id.source == "tvdb" && id.value == "415677"),
        "hydrated title should preserve the Bastard!! TVDB identity"
    );
}

#[tokio::test]
async fn library_anime_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "567890".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .catalog
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Existing Anime [BD]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Anime</title><tvdbid>567890</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Anime.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let mut linked_files = Vec::new();
    for _ in 0..100 {
        linked_files = ctx
            .library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Anime");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Series",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "345678".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .catalog
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Existing Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Series</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let mut linked_files = Vec::new();
    for _ in 0..100 {
        linked_files = ctx
            .library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Series");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_existing_unhydrated_title_without_episodes_completes_session() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = ctx
        .catalog
        .create(Title {
            id: Id::new().0,
            name: "Pending Series".to_string(),
            facet: MediaFacet::Series,
            monitored: false,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "345679".to_string(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: Some("Pending hydration title".to_string()),
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Pending Series".to_string()),
            slug: Some("pending-series".to_string()),
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
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        })
        .await
        .expect("create pending title");

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Pending Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Pending Series</title><tvdbid>345679</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Pending.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let refreshed_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let episodes = ctx
        .catalog
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        ctx.app.active_library_scan_sessions().await.is_empty(),
        "scan session should complete when an existing unhydrated title is skipped",
    );
    assert!(media_files.is_empty());
    assert!(episodes.is_empty());
}

#[tokio::test]
async fn library_series_scan_creates_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Bluey");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Bluey</title><tvdbid>81189</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let titles = ctx
        .catalog
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].name, "Bluey");
    assert!(!titles[0].monitored);
}

#[tokio::test]
async fn library_series_scan_counts_new_title_files_before_post_hydration_scan_progress() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Bluey");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Bluey</title><tvdbid>81189</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    std::fs::write(
        season_dir.join("Bluey.S01E01.720p.WEB-DL.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake episode");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);

    assert!(
        ctx.app.active_library_scan_sessions().await.is_empty(),
        "scan session should complete before the synchronous scan call returns",
    );
}

#[tokio::test]
async fn library_movie_scan_refreshes_existing_title_from_disk_without_renaming() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let movie_dir = media_root.path().join("Existing Movie [2160p]");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Existing.Movie.2024.2160p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Existing Movie</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let refreshed_title = ctx
        .catalog
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Movie");

    let collections = ctx
        .catalog
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );

    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        movie_path.to_string_lossy().to_string()
    );
    assert_eq!(media_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_movie_scan_creates_unmonitored_title_and_collection() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = load_fixture("smg/get_movie.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let movie_dir = media_root.path().join("Test Movie Title (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Test.Movie.Title.2024.1080p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Test Movie Title</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .catalog
            .list(Some(MediaFacet::Movie), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let hydrated_title = hydrated_title.expect("movie title should hydrate");
    assert_eq!(hydrated_title.name, "Test Movie Title");
    assert!(!hydrated_title.monitored);

    let collections = ctx
        .catalog
        .list_collections_for_title(&hydrated_title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert!(!collections[0].monitored);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );

    let media_files = ctx
        .library_state
        .list_media_files_for_title(&hydrated_title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        movie_path.to_string_lossy().to_string()
    );
    assert_eq!(media_files[0].scan_status, "scan_failed");

    let (wanted_items, total) = ctx
        .app
        .list_wanted_items(scryer_application::WantedItemsQuery {
            status: None,
            media_type: None,
            title_id: Some(hydrated_title.id.clone()),
            title_search: None,
            latest_decision_code: None,
            limit: 10,
            offset: 0,
        })
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_series_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let folder = media_root.path().join(format!("Show {index:04}"));
        std::fs::create_dir_all(&folder).expect("create show dir");
        std::fs::write(
            folder.join("tvshow.nfo"),
            format!(
                "<tvshow><title>Show {index:04}</title><tvdbid>{}</tvdbid></tvshow>",
                900_000 + index
            ),
        )
        .expect("write tvshow.nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .catalog
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

#[tokio::test]
async fn library_movie_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let display_name = format!("Movie.Title.{index:04}.2024");
        let video_path = media_root.path().join(format!("{display_name}.mkv"));
        std::fs::write(&video_path, b"video").expect("write movie");
        std::fs::write(
            video_path.with_extension("nfo"),
            format!(
                "<movie><title>Movie {index:04}</title><tvdbid>{}</tvdbid><year>2024</year></movie>",
                800_000 + index
            ),
        )
        .expect("write movie nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.matched, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .catalog
        .list(Some(MediaFacet::Movie), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

#[tokio::test]
async fn graphql_delete_title() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "To Delete", "movie").await;

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"], true);

    // Verify deleted
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id } }"#,
        json!({ "id": id }),
    )
    .await;
    assert!(body["data"]["title"].is_null(), "title should be gone");
}

#[tokio::test]
async fn graphql_delete_title_cleans_title_workflow_state() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Delete With Cleanup", "movie").await;

    ctx.library_state
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: id.clone(),
            title_name: Some("Delete With Cleanup".to_string()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: "auto".to_string(),
            next_search_at: None,
            last_search_at: None,
            search_count: 0,
            baseline_date: None,
            status: scryer_application::WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2026-03-12T00:00:00Z".to_string(),
            updated_at: "2026-03-12T00:00:00Z".to_string(),
        })
        .await
        .expect("seed wanted item");
    ctx.db
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: "wanted-delete".to_string(),
            title_id: id.clone(),
            release_title: "Delete With Cleanup 2026".to_string(),
            release_url: Some("https://example.invalid/release.nzb".to_string()),
            source_kind: None,
            release_size_bytes: Some(1_024),
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("test-indexer".to_string()),
            release_guid: Some("guid-delete".to_string()),
            added_at: "2026-03-12T00:00:00Z".to_string(),
            delay_until: "2026-03-13T00:00:00Z".to_string(),
            status: scryer_application::PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed pending release");
    let workflow_store = SqliteWorkflowStore::new(&ctx.db);
    workflow_store
        .record_submission(scryer_application::DownloadSubmission {
            title_id: id.clone(),
            facet: "movie".to_string(),
            download_client_id: None,
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "queue-delete".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Delete With Cleanup".to_string()),
            request_signature: None,
            scope: scryer_application::SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"], true);

    assert!(
        ctx.db
            .list_wanted_items(scryer_application::WantedItemsQuery {
                title_id: Some(id.clone()),
                limit: 10,
                ..scryer_application::WantedItemsQuery::default()
            })
            .await
            .expect("wanted items")
            .is_empty()
    );
    assert!(
        ctx.db
            .list_waiting_pending_releases()
            .await
            .expect("pending releases")
            .iter()
            .all(|entry| entry.title_id != id)
    );
    assert!(
        workflow_store
            .list_for_title(&id)
            .await
            .expect("download submissions")
            .is_empty()
    );
}

#[tokio::test]
async fn graphql_filter_titles_by_facet() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie A", "movie").await;
    add_test_title(&ctx, "Series A", "series").await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { name facet } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["facet"], "movie");
}

#[tokio::test]
async fn graphql_series_titles_expose_series_facet() {
    let ctx = TestContext::new().await;
    let expected_name = "Series A";
    add_test_title(&ctx, expected_name, "series").await;

    let body = gql(&ctx, "{ titles { name facet } }", json!({})).await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    let title = &titles[0];
    assert_eq!(title["name"], expected_name);
    assert_eq!(title["facet"], "series");
}

// ---------------------------------------------------------------------------
// User management
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_me_query() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ me { id username } }", json!({})).await;
    assert_no_errors(&body);
    // auth-disabled mode creates an "admin" user
    assert_eq!(body["data"]["me"]["username"], "admin");
}

#[tokio::test]
async fn graphql_users_query() {
    let ctx = TestContext::new().await;
    // Trigger default admin user creation first
    gql(&ctx, "{ me { id } }", json!({})).await;

    let body = gql(&ctx, "{ users { id username } }", json!({})).await;
    assert_no_errors(&body);
    let users = body["data"]["users"].as_array().unwrap();
    assert!(!users.is_empty(), "should have at least one user");
}

#[tokio::test]
async fn graphql_create_user() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "testuser", "password": "testpass123", "entitlements": [] } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createUser"]["username"], "testuser");
}

#[tokio::test]
async fn graphql_dev_auto_login() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { devAutoLogin { token user { username } } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["devAutoLogin"]["token"].is_string(),
        "should return token"
    );
    assert_eq!(body["data"]["devAutoLogin"]["user"]["username"], "admin");
}

// ---------------------------------------------------------------------------
// Download queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_download_queue_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadQueue { id titleName } }", json!({})).await;
    assert_no_errors(&body);
    let queue = body["data"]["downloadQueue"].as_array().unwrap();
    assert!(queue.is_empty(), "queue should start empty");
}

#[tokio::test]
async fn graphql_invalid_nzb_xml_queue_failure_is_blocklisted() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Broken NZB Movie", "movie").await;
    let source_hint = format!("{}/invalid.nzb", ctx.nzbget_server.uri());
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let candidate_token = ctx
        .app
        .issue_release_candidate_token(
            &admin,
            &title_id,
            &scryer_application::SubmissionScope::Title,
            &scryer_application::QueuedReleaseSelection {
                source_hint: Some(source_hint.clone()),
                source_kind: Some(scryer_application::DownloadSourceKind::NzbFile),
                source_title: Some("Broken.NZB.Movie.2024".to_string()),
            },
        )
        .await
        .expect("issue candidate token");

    Mock::given(method("GET"))
        .and(path("/invalid.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not xml"))
        .mount(&ctx.nzbget_server)
        .await;

    let queue_body = gql(
        &ctx,
        r#"
        mutation($input: QueueDownloadInput!) {
          queueExistingTitleDownload(input: $input) {
            jobId
          }
        }
        "#,
        json!({
            "input": {
                "titleId": title_id,
                "candidateToken": candidate_token,
                "scope": { "title": true },
            }
        }),
    )
    .await;

    assert!(
        queue_body.get("errors").is_some(),
        "expected queue mutation to fail for invalid nzb xml: {queue_body}"
    );
    let error_message = queue_body["errors"][0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        error_message.contains("did not look like xml")
            || error_message.contains("root element must be <nzb>")
            || error_message.contains("not valid xml"),
        "expected invalid-xml error message, got: {error_message}"
    );

    let blocklist_body = gql(
        &ctx,
        r#"
        query($titleId: String!) {
          titleReleaseBlocklist(titleId: $titleId) {
            sourceHint
            sourceTitle
            errorMessage
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&blocklist_body);
    let entries = blocklist_body["data"]["titleReleaseBlocklist"]
        .as_array()
        .expect("blocklist entries array");
    assert!(
        entries.iter().any(|entry| {
            entry["sourceHint"].as_str() == Some(source_hint.as_str())
                && entry["sourceTitle"].as_str() == Some("Broken.NZB.Movie.2024")
                && entry["errorMessage"].as_str().is_some_and(|message| {
                    message.contains("did not look like xml")
                        || message.contains("root element must be <nzb>")
                        || message.contains("not valid xml")
                })
        }),
        "expected invalid nzb release to appear in titleReleaseBlocklist: {blocklist_body}"
    );
}

#[tokio::test]
async fn graphql_download_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ downloadHistory(limit: 50, offset: 0) { items { id titleName } hasMore } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let items = body["data"]["downloadHistory"]["items"].as_array().unwrap();
    assert!(items.is_empty(), "history should start empty");
    assert_eq!(body["data"]["downloadHistory"]["hasMore"], json!(false));
}

#[tokio::test]
async fn graphql_run_housekeeping_reports_pruned_staged_nzb_artifacts() {
    let ctx = TestContext::new().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged = ctx
        .staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");
    ctx.staged_nzb_store
        .set_staged_nzb_updated_at(&staged, Utc::now() - Duration::hours(2))
        .await
        .expect("staged artifact timestamp should update");

    let body = gql(
        &ctx,
        "mutation { runHousekeeping { stagedNzbArtifactsPruned } }",
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["runHousekeeping"]["stagedNzbArtifactsPruned"],
        1
    );
    assert_eq!(
        ctx.staged_nzb_store.count_staged_artifacts().await.unwrap(),
        0
    );
}

#[tokio::test]
async fn graphql_run_housekeeping_respects_configured_history_retention() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app
        .find_or_create_default_user()
        .await
        .expect("default user should initialize");
    let baseline_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("baseline domain events count");

    let title = create_catalog_title(
        &ctx,
        "Retention Fixture",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "12345".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let now = Utc::now();
    let stale_at = (now - Duration::days(40)).to_rfc3339();
    let very_stale_at = (now - Duration::days(120)).to_rfc3339();
    let fresh_at = (now - Duration::days(5)).to_rfc3339();
    let wanted_item_id = Id::new().0;
    let stale_completed_import_id = Id::new().0;
    let fresh_completed_import_id = Id::new().0;
    let stale_processing_import_id = Id::new().0;

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, media_type, search_phase, status, created_at, updated_at)
         VALUES (?, ?, NULL, 'series', 'primary', 'wanted', ?, ?)",
    )
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("wanted item should insert");

    sqlx::query(
        "INSERT INTO release_decisions
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes, decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
         VALUES (?, ?, ?, 'stale-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?),
                (?, ?, ?, 'fresh-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("release decisions should insert");

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, attempted_at, created_at, updated_at)
         VALUES (?, ?, NULL, 'stale-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, ?, NULL, 'fresh-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, ?, NULL, 'pending-attempt', 'pending', NULL, ?, ?, ?)",
    )
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&very_stale_at)
    .bind(&very_stale_at)
    .bind(&very_stale_at)
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("release attempts should insert");

    sqlx::query(
        "INSERT INTO history_events
         (id, event_type, actor_user_id, title_id, message, occurred_at, source, created_at, metadata_json)
         VALUES (?, 'test', NULL, NULL, 'stale-history', ?, NULL, ?, NULL),
                (?, 'test', NULL, NULL, 'fresh-history', ?, NULL, ?, NULL)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("history events should insert");

    sqlx::query(
        "INSERT INTO domain_events
         (event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json)
         VALUES (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'title_added', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'import_requested', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'library_scan_progressed', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'job_run_started', '{}')",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("domain events should insert");

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, 'test', 'stale-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?),
                (?, 'test', 'fresh-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?),
                (?, 'test', 'stale-processing', 'manual_import', 'processing', '{}', NULL, NULL, NULL, ?, ?)",
    )
    .bind(&stale_completed_import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&fresh_completed_import_id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&stale_processing_import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("imports should insert");

    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name, media_kind, title_id, episode_id, season_number, episode_number, result, reason_code, imported_media_file_id, created_at)
         VALUES (?, 'test', 'stale-completed', ?, NULL, 'stale.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?),
                (?, 'test', 'fresh-completed', ?, NULL, 'fresh.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?),
                (?, 'test', 'stale-processing', ?, NULL, 'active.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_completed_import_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_completed_import_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&stale_processing_import_id)
    .bind(&title.id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("download import artifacts should insert");

    sqlx::query(
        "INSERT INTO rule_set_history (id, rule_set_id, action, rego_source, actor_id, created_at)
         VALUES (?, 'rule-1', 'updated', NULL, NULL, ?),
                (?, 'rule-1', 'updated', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("rule set history should insert");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 30
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let body = gql(
        &ctx,
        "mutation { runHousekeeping { staleReleaseDecisions staleReleaseAttempts staleHistoryEvents staleHistoryRecords } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["runHousekeeping"]["staleReleaseDecisions"], 1);
    assert_eq!(body["data"]["runHousekeeping"]["staleReleaseAttempts"], 1);
    assert_eq!(body["data"]["runHousekeeping"]["staleHistoryEvents"], 1);
    assert_eq!(body["data"]["runHousekeeping"]["staleHistoryRecords"], 8);

    let remaining_release_decisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_decisions")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release decisions count");
    assert_eq!(remaining_release_decisions, 1);

    let remaining_release_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_download_attempts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release attempts count");
    assert_eq!(remaining_release_attempts, 2);

    let remaining_history_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("history events count");
    assert_eq!(remaining_history_events, 1);

    let remaining_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("domain events count");
    assert_eq!(remaining_domain_events, baseline_domain_events + 3);

    let remaining_imports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(ctx.db.pool())
        .await
        .expect("imports count");
    assert_eq!(remaining_imports, 2);

    let remaining_import_artifacts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_import_artifacts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("download import artifacts count");
    assert_eq!(remaining_import_artifacts, 2);

    let remaining_rule_set_history: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rule_set_history")
            .fetch_one(ctx.db.pool())
            .await
            .expect("rule set history count");
    assert_eq!(remaining_rule_set_history, 1);
}

#[tokio::test]
async fn graphql_run_housekeeping_skips_history_retention_when_keep_forever_is_enabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let baseline_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("baseline domain events count");
    let stale_at = (Utc::now() - Duration::days(400)).to_rfc3339();
    let stale_attempt_at = (Utc::now() - Duration::days(120)).to_rfc3339();
    let import_id = Id::new().0;
    let title = create_catalog_title(
        &ctx,
        "Retention Keep Forever Fixture",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "67890".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let wanted_item_id = Id::new().0;

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, media_type, search_phase, status, created_at, updated_at)
         VALUES (?, ?, NULL, 'series', 'primary', 'wanted', ?, ?)",
    )
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("wanted item should insert");

    sqlx::query(
        "INSERT INTO history_events
         (id, event_type, actor_user_id, title_id, message, occurred_at, source, created_at, metadata_json)
         VALUES (?, 'test', NULL, NULL, 'stale-history', ?, NULL, ?, NULL)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("history event should insert");

    sqlx::query(
        "INSERT INTO domain_events
         (event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json)
         VALUES (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'title_added', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'library_scan_progressed', '{}')",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("domain events should insert");

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, 'test', 'stale-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?)",
    )
    .bind(&import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("import should insert");

    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name, media_kind, title_id, episode_id, season_number, episode_number, result, reason_code, imported_media_file_id, created_at)
         VALUES (?, 'test', 'stale-completed', ?, NULL, 'stale.mkv', 'episode', NULL, NULL, NULL, NULL, 'imported', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&import_id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("download import artifact should insert");

    sqlx::query(
        "INSERT INTO release_decisions
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes, decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
         VALUES (?, ?, ?, 'stale-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("release decision should insert");

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, attempted_at, created_at, updated_at)
         VALUES (?, NULL, NULL, 'stale-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, NULL, NULL, 'pending-attempt', 'pending', NULL, ?, ?, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(Id::new().0)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .execute(ctx.db.pool())
    .await
    .expect("release attempts should insert");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": true,
            "historyRetentionDays": 180
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let body = gql(
        &ctx,
        "mutation { runHousekeeping { staleReleaseDecisions staleReleaseAttempts staleHistoryEvents staleHistoryRecords } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["runHousekeeping"]["staleReleaseDecisions"], 1);
    assert_eq!(body["data"]["runHousekeeping"]["staleReleaseAttempts"], 1);
    assert_eq!(body["data"]["runHousekeeping"]["staleHistoryEvents"], 0);
    assert_eq!(body["data"]["runHousekeeping"]["staleHistoryRecords"], 3);

    let remaining_history_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("history events count");
    assert_eq!(remaining_history_events, 1);

    let remaining_imports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(ctx.db.pool())
        .await
        .expect("imports count");
    assert_eq!(remaining_imports, 1);

    let remaining_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("domain events count");
    assert_eq!(remaining_domain_events, baseline_domain_events + 2);

    let remaining_import_artifacts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_import_artifacts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("download import artifacts count");
    assert_eq!(remaining_import_artifacts, 1);

    let remaining_release_decisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_decisions")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release decisions count");
    assert_eq!(remaining_release_decisions, 0);

    let remaining_release_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_download_attempts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release attempts count");
    assert_eq!(remaining_release_attempts, 1);
}

#[tokio::test]
async fn sqlite_history_retention_indexes_exist_after_migrations() {
    let ctx = TestContext::new().await;

    let history_event_indexes = sqlx::query("PRAGMA index_list('history_events')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("history event indexes");
    let history_event_index_names = history_event_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(history_event_index_names.contains(&"idx_history_events_occurred_at".to_string()));

    let import_indexes = sqlx::query("PRAGMA index_list('imports')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("import indexes");
    let import_index_names = import_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(import_index_names.contains(&"idx_imports_status_updated_at".to_string()));

    let rule_set_history_indexes = sqlx::query("PRAGMA index_list('rule_set_history')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("rule set history indexes");
    let rule_set_history_index_names = rule_set_history_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(rule_set_history_index_names.contains(&"idx_rule_set_history_created_at".to_string()));

    let release_decision_indexes = sqlx::query("PRAGMA index_list('release_decisions')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("release decision indexes");
    let release_decision_index_names = release_decision_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(release_decision_index_names.contains(&"idx_release_decisions_created_at".to_string()));

    let import_artifact_indexes = sqlx::query("PRAGMA index_list('download_import_artifacts')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("download import artifact indexes");
    let import_artifact_index_names = import_artifact_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(
        import_artifact_index_names
            .contains(&"idx_download_import_artifacts_retention".to_string())
    );
}

// ---------------------------------------------------------------------------
// System health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_system_health() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ systemHealth { serviceReady totalTitles } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["systemHealth"]["serviceReady"].is_boolean(),
        "should return serviceReady boolean"
    );
}

#[tokio::test]
async fn graphql_smg_version_compatibility_notice_reads_persisted_notice() {
    let ctx = TestContext::new().await;
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "service".into(),
            scope: "system".into(),
            key_name: "smg.version_compatibility_notice".into(),
            data_type: "json".into(),
            default_value_json: "null".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("compatibility notice definition should seed");
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "smg.version_compatibility_notice",
            None,
            json!({
                "status": "deprecated",
                "minimum_version": "0.14.2",
                "your_version": "0.14.1",
                "message": "Upgrade before support ends.",
                "upgrade_deadline": "2026-06-01",
            })
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("compatibility notice should persist");

    let body = gql(
        &ctx,
        r#"{ smgVersionCompatibilityNotice { status minimumVersion yourVersion message upgradeDeadline } }"#,
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["status"],
        "deprecated"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["minimumVersion"],
        "0.14.2"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["yourVersion"],
        "0.14.1"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["message"],
        "Upgrade before support ends."
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["upgradeDeadline"],
        "2026-06-01"
    );
}

// ---------------------------------------------------------------------------
// Activity / events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_activity_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ activityEvents { id kind severity } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["activityEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleEvents { id eventType sourceTitle quality occurredAt } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["titleEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleHistory(filter: { limit: 10 }) { records { id eventType sourceTitle } totalCount } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 0);
    assert!(body["data"]["titleHistory"]["records"].is_array());
}

#[tokio::test]
async fn graphql_title_history_works_without_legacy_table() {
    let ctx = TestContext::new().await;
    let legacy_table: Option<String> = sqlx::query_scalar(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
            AND name = 'title_history'
          LIMIT 1",
    )
    .fetch_optional(ctx.db.pool())
    .await
    .expect("sqlite master query should succeed");
    assert_eq!(legacy_table, None);

    let title = create_catalog_title(
        &ctx,
        "Legacy Title History Fixture",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: String!) {
          titleHistory(filter: { titleIds: [$titleId], limit: 10 }) {
            totalCount
            records {
              id
              eventType
              sourceTitle
            }
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 0);
    assert_eq!(
        body["data"]["titleHistory"]["records"]
            .as_array()
            .expect("history records array")
            .len(),
        0
    );
}

#[tokio::test]
async fn graphql_title_history_rejects_unsupported_event_type_filters() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleHistory(filter: { eventTypes: ["download_completed"], limit: 10 }) { totalCount } }"#,
        json!({}),
    )
    .await;

    let errors = body["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(message.contains("unsupported title history event type `download_completed`"));
    assert!(message.contains("grabbed"));
    assert!(message.contains("rematched"));
}

#[tokio::test]
async fn graphql_title_events_rejects_unsupported_event_type_filters() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleEvents(eventTypes: ["download_ignored"], limit: 10) { id eventType } }"#,
        json!({}),
    )
    .await;

    let errors = body["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(message.contains("unsupported title history event type `download_ignored`"));
    assert!(message.contains("imported"));
    assert!(message.contains("rematched"));
}

#[tokio::test]
async fn graphql_title_history_includes_download_failed_and_blocklisted_events() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Download Outcome History Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("download-outcome-episode-1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let title_context = TitleContextSnapshot {
        title_name: title.name.clone(),
        facet: title.facet,
        external_ids: DomainExternalIds::default(),
        poster_url: title.poster_url.clone(),
        year: title.year,
    };

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                title: Some(title_context.clone()),
                source_title: Some("Fixture.S01.1080p.WEB-DL".to_string()),
                source_hint: Some("https://indexer.example/release".to_string()),
                download_id: Some("job-123".to_string()),
                client_id: Some("client-1".to_string()),
                client_name: Some("Primary".to_string()),
                client_type: Some("nzbget".to_string()),
                quality: Some("1080P".to_string()),
                reason: Some(
                    "download failed for 'Fixture.S01.1080p.WEB-DL': CORRUPT ARCHIVE".to_string(),
                ),
                episode_ids: vec![episode.id.clone()],
                collection_id: Some(collection.id.clone()),
            }),
        })
        .await
        .expect("append download failed event");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ReleaseBlocklisted(ReleaseBlocklistedEventData {
                title: Some(title_context),
                source_title: Some("Fixture.S01.1080p.WEB-DL".to_string()),
                source_hint: Some("https://indexer.example/release".to_string()),
                download_id: Some("job-123".to_string()),
                client_id: Some("client-1".to_string()),
                client_name: Some("Primary".to_string()),
                client_type: Some("nzbget".to_string()),
                quality: Some("1080P".to_string()),
                reason: Some("download client failure: CORRUPT ARCHIVE".to_string()),
                episode_ids: vec![episode.id.clone()],
                collection_id: Some(collection.id.clone()),
            }),
        })
        .await
        .expect("append release blocklisted event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: String!) {
          titleHistory(filter: { titleIds: [$titleId], eventTypes: ["download_failed", "blocklisted"], limit: 10 }) {
            totalCount
            records {
              eventType
              sourceTitle
              downloadId
              clientId
              clientName
              failureReason
              blocklistReason
              episodeId
              collectionId
            }
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["titleHistory"]["totalCount"], 2);
    let records = body["data"]["titleHistory"]["records"]
        .as_array()
        .expect("title history records array");

    let download_failed = records
        .iter()
        .find(|record| record["eventType"] == "download_failed")
        .expect("download_failed record");
    assert_eq!(download_failed["sourceTitle"], "Fixture.S01.1080p.WEB-DL");
    assert_eq!(download_failed["downloadId"], "job-123");
    assert_eq!(download_failed["clientId"], "client-1");
    assert_eq!(download_failed["clientName"], "Primary");
    assert_eq!(download_failed["episodeId"], episode.id);
    assert_eq!(download_failed["collectionId"], collection.id);
    assert!(
        download_failed["failureReason"]
            .as_str()
            .is_some_and(|value| value.contains("CORRUPT ARCHIVE"))
    );

    let blocklisted = records
        .iter()
        .find(|record| record["eventType"] == "blocklisted")
        .expect("blocklisted record");
    assert_eq!(blocklisted["downloadId"], "job-123");
    assert_eq!(blocklisted["clientId"], "client-1");
    assert_eq!(blocklisted["clientName"], "Primary");
    assert_eq!(blocklisted["episodeId"], episode.id);
    assert_eq!(blocklisted["collectionId"], collection.id);
    assert_eq!(
        blocklisted["blocklistReason"],
        "download client failure: CORRUPT ARCHIVE"
    );
}

#[tokio::test]
async fn graphql_title_history_filters_by_episode_id() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Episode Scoped History Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode_one = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("episode-history-1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create first episode");
    let episode_two = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode Two".to_string()),
            air_date: Some("2024-01-08".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("2".to_string()),
            overview: None,
            tvdb_id: Some("episode-history-2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create second episode");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                },
                media_updates: vec![
                    MediaPathUpdate {
                        path: "/library/Episode Scoped History Fixture/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                    MediaPathUpdate {
                        path: "/library/Episode Scoped History Fixture/S01E02.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                ],
                imported_count: 2,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: None,
                source_path: None,
                dest_path: None,
                quality: None,
                episode_ids: vec![episode_one.id.clone(), episode_two.id.clone()],
            }),
        })
        .await
        .expect("append import completed event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: String!, $episodeId: String!) {
          titleHistory(filter: { titleIds: [$titleId], episodeId: $episodeId, limit: 10 }) {
            totalCount
            records {
              eventType
              episodeId
            }
          }
        }
        "#,
        json!({ "titleId": title.id, "episodeId": episode_one.id }),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["titleHistory"]["totalCount"], 1);
    let records = body["data"]["titleHistory"]["records"]
        .as_array()
        .expect("title history records array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["eventType"], "imported");
    assert_eq!(records[0]["episodeId"], episode_one.id);
}

#[tokio::test]
async fn graphql_episode_history_omits_ambiguous_source_path_for_multi_file_events() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "History Projection Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode_one = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("history-episode-1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create first episode");
    let episode_two = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode Two".to_string()),
            air_date: Some("2024-01-08".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("2".to_string()),
            overview: None,
            tvdb_id: Some("history-episode-2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create second episode");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                },
                media_updates: vec![
                    MediaPathUpdate {
                        path: "/library/History Projection Fixture/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                    MediaPathUpdate {
                        path: "/library/History Projection Fixture/S01E02.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                ],
                imported_count: 2,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: None,
                source_path: None,
                dest_path: None,
                quality: None,
                episode_ids: vec![episode_one.id.clone(), episode_two.id.clone()],
            }),
        })
        .await
        .expect("append import completed event");

    let body = gql(
        &ctx,
        r#"
        query EpisodeHistory($episodeId: String!) {
          episodeHistory(episodeId: $episodeId, limit: 10) {
            eventType
            sourceTitle
          }
        }
        "#,
        json!({ "episodeId": episode_one.id }),
    )
    .await;
    assert_no_errors(&body);

    let records = body["data"]["episodeHistory"]
        .as_array()
        .expect("episode history array");
    let imported = records
        .iter()
        .find(|record| record["eventType"] == "imported")
        .expect("imported event");
    assert!(imported["sourceTitle"].is_null());
}

// ---------------------------------------------------------------------------
// Metadata queries (via SMG mock)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_search_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_tvdb_rich.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: String!) {
            searchMetadata(query: $query, type: $type) {
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "Test Movie Title");
}

#[tokio::test]
async fn graphql_search_metadata_movie_accepts_year_hint() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_tvdb_rich.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param(
            "variables",
            r#"{"query":"Test Movie","type":"movie","limit":25,"language":"eng","year":2024}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: String!, $year: Int) {
            searchMetadata(query: $query, type: $type, year: $year) {
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "movie", "year": 2024 }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "Test Movie Title");
}

#[tokio::test]
async fn graphql_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/get_movie.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($tvdbId: Int!) {
            metadataMovie(tvdbId: $tvdbId) {
                name year runtimeMinutes genres overview
            }
        }"#,
        json!({ "tvdbId": 123456 }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["metadataMovie"]["name"], "Test Movie Title");
    assert_eq!(body["data"]["metadataMovie"]["year"], 2024);
    assert_eq!(body["data"]["metadataMovie"]["runtimeMinutes"], 142);
}

#[tokio::test]
async fn graphql_metadata_series() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/get_series.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            metadataSeries(id: $id) {
                name year seasons { number label } episodes { name seasonNumber }
            }
        }"#,
        json!({ "id": "345678" }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "Test Show Name");
    assert_eq!(series["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(series["episodes"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_fix_title_match_movie_updates_identity_and_history() {
    let ctx = TestContext::new().await;
    mount_smg_mocks(&ctx, "smg/get_movie.json").await;

    let title = create_catalog_title(
        &ctx,
        "Broken Movie Match",
        MediaFacet::Movie,
        vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "999".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt0000999".to_string(),
            },
            ExternalId {
                source: "tmdb".to_string(),
                value: "4444".to_string(),
            },
        ],
        vec!["scryer:quality-profile:4k".to_string()],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan { scanned }
            title {
              id
              name
              slug
              imdbId
              metadataFetchedAt
              tags
              externalIds { source value }
            }
          }
        }
        "#,
        json!({ "input": { "titleId": title.id, "tvdbId": "123456" } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["fixTitleMatch"];
    assert_eq!(payload["hydrated"], true);
    assert_eq!(payload["warnings"], json!([]));
    assert!(payload["libraryScan"].is_null());
    assert_eq!(payload["title"]["name"], "Test Movie Title");
    assert_eq!(payload["title"]["slug"], "test-movie-title");
    assert_eq!(payload["title"]["imdbId"], "tt1234567");
    assert!(payload["title"]["metadataFetchedAt"].is_string());

    let tags = payload["title"]["tags"].as_array().expect("tags array");
    assert!(tags.contains(&json!("scryer:quality-profile:4k")));

    let external_ids = payload["title"]["externalIds"]
        .as_array()
        .expect("external ids array");
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "123456" })
    );
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "imdb" && value["value"] == "tt1234567" })
    );
    assert!(
        !external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "999" })
    );
    assert!(!external_ids.iter().any(|value| value["source"] == "tmdb"));

    let events = gql(
        &ctx,
        r#"
        query TitleEvents($titleId: String!) {
          titleEvents(titleId: $titleId, limit: 10) {
            eventType
            dataJson
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&events);
    let rematch_events = events["data"]["titleEvents"]
        .as_array()
        .expect("title events array");
    let rematch_event = rematch_events
        .iter()
        .find(|event| event["eventType"] == "rematched")
        .expect("rematched history event");
    let data_json = rematch_event["dataJson"]
        .as_str()
        .expect("rematch data json");
    let data_value: Value = serde_json::from_str(data_json).expect("parse rematch data");
    assert_eq!(data_value["old_tvdb_id"], "999");
    assert_eq!(data_value["new_tvdb_id"], "123456");
    assert_eq!(data_value["source"], "manual");

    let history = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: String!) {
          titleHistory(filter: { titleIds: [$titleId], eventTypes: ["rematched"], limit: 10 }) {
            totalCount
            records {
              eventType
            }
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&history);
    assert_eq!(history["data"]["titleHistory"]["totalCount"], 1);
    assert_eq!(
        history["data"]["titleHistory"]["records"][0]["eventType"],
        "rematched"
    );

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_started")
    );
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_completed")
    );
    assert!(activity_kinds.iter().any(|kind| kind == "title_updated"));
}

#[tokio::test]
async fn graphql_fix_title_match_series_rebuilds_and_relinks_library() {
    let ctx = TestContext::new().await;
    mount_smg_mocks(&ctx, "smg/get_series.json").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let title_name = "Broken Series Match";
    let title = create_catalog_title(
        &ctx,
        title_name,
        MediaFacet::Series,
        vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "999".to_string(),
            },
            ExternalId {
                source: "mal".to_string(),
                value: "5555".to_string(),
            },
        ],
        vec![
            format!("scryer:root-folder:{}", media_root.path().display()),
            "scryer:season-folder:enabled".to_string(),
            "scryer:anime-status:finished".to_string(),
        ],
        true,
    )
    .await;

    let old_collection = ctx
        .catalog
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "99".to_string(),
            label: Some("Legacy Season".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create old collection");

    let old_episode = ctx
        .catalog
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(old_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("99".to_string()),
            episode_label: Some("S99E01".to_string()),
            title: Some("Legacy Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: Some("Legacy episode".to_string()),
            tvdb_id: Some("9999001".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create old episode");

    let season_dir = media_root.path().join(title_name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Broken.Series.Match.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");
    let file_id = ctx
        .library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &old_episode.id)
        .await
        .expect("link file to legacy episode");

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan {
              scanned
              matched
              imported
              skipped
              unmatched
            }
            title {
              id
              name
              tags
              externalIds { source value }
              collections {
                id
                collectionIndex
                episodes {
                  id
                  seasonNumber
                  episodeNumber
                  title
                }
              }
              mediaFiles {
                episodeId
                filePath
              }
            }
          }
        }
        "#,
        json!({ "input": { "titleId": title.id, "tvdbId": "345678" } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["fixTitleMatch"];
    assert_eq!(payload["hydrated"], true);
    assert_eq!(payload["warnings"], json!([]));
    assert_eq!(payload["title"]["name"], "Test Show Name");
    assert_eq!(payload["libraryScan"]["scanned"], 1);
    assert_eq!(payload["libraryScan"]["unmatched"], 0);

    let tags = payload["title"]["tags"].as_array().expect("tags array");
    assert!(tags.contains(&json!(format!(
        "scryer:root-folder:{}",
        media_root.path().display()
    ))));
    assert!(tags.contains(&json!("scryer:season-folder:enabled")));
    assert!(!tags.contains(&json!("scryer:anime-status:finished")));

    let external_ids = payload["title"]["externalIds"]
        .as_array()
        .expect("external ids array");
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "345678" })
    );
    assert!(!external_ids.iter().any(|value| value["source"] == "mal"));

    let collections = payload["title"]["collections"]
        .as_array()
        .expect("collections array");
    assert_eq!(collections.len(), 2);
    assert!(
        !collections
            .iter()
            .any(|collection| collection["id"] == old_collection.id)
    );
    let rebuilt_episode_count: usize = collections
        .iter()
        .map(|collection| {
            collection["episodes"]
                .as_array()
                .expect("episodes array")
                .len()
        })
        .sum();
    assert_eq!(rebuilt_episode_count, 3);

    let media_files = payload["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
    let relinked_episode_id = media_files[0]["episodeId"]
        .as_str()
        .expect("media file should relink to rebuilt episode");
    assert_ne!(relinked_episode_id, old_episode.id);

    let events = gql(
        &ctx,
        r#"
        query TitleEvents($titleId: String!) {
          titleEvents(titleId: $titleId, limit: 10) {
            eventType
            dataJson
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&events);
    let rematch_events = events["data"]["titleEvents"]
        .as_array()
        .expect("title events array");
    let rematch_event = rematch_events
        .iter()
        .find(|event| event["eventType"] == "rematched")
        .expect("rematched history event");
    let data_json = rematch_event["dataJson"]
        .as_str()
        .expect("rematch data json");
    let data_value: Value = serde_json::from_str(data_json).expect("parse rematch data");
    assert_eq!(data_value["old_tvdb_id"], "999");
    assert_eq!(data_value["new_tvdb_id"], "345678");

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_started")
    );
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_completed")
    );
    assert!(activity_kinds.iter().any(|kind| kind == "title_updated"));
}

#[tokio::test]
async fn graphql_fix_title_match_rejects_duplicate_target_tvdb_id() {
    let ctx = TestContext::new().await;
    let existing = create_catalog_title(
        &ctx,
        "Existing Correct Match",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let broken = create_catalog_title(
        &ctx,
        "Broken Match",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "999".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            title { id }
          }
        }
        "#,
        json!({ "input": { "titleId": broken.id, "tvdbId": "123456" } }),
    )
    .await;

    assert!(
        body.get("errors").is_some(),
        "expected graphql errors: {body}"
    );
    let message = body["errors"][0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(message.contains("tvdb id 123456 is already assigned to title"));
    assert!(message.contains(&existing.name));
}

// ---------------------------------------------------------------------------
// Configuration (indexers + download clients)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_indexers_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ indexers { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["indexers"].is_array());
}

#[tokio::test]
async fn graphql_download_client_configs_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadClientConfigs { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["downloadClientConfigs"].is_array());
}

// ---------------------------------------------------------------------------
// Wanted items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_wanted_items_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($status: WantedStatusValue, $mediaType: WantedMediaTypeValue) {
            wantedItems(status: $status, mediaType: $mediaType) {
                items { id }
                total
            }
        }"#,
        json!({ "status": "wanted", "mediaType": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["wantedItems"]["total"], 0,
        "should have no wanted items initially"
    );
}

// ---------------------------------------------------------------------------
// Rule sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_rule_sets_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Import history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_import_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ importHistory { id sourceTitle status } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["importHistory"].is_array());
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_calendar_episodes() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($start: String!, $end: String!) {
            calendarEpisodes(startDate: $start, endDate: $end) {
                episodeTitle seasonNumber episodeNumber
            }
        }"#,
        json!({ "start": "2024-01-01", "end": "2024-12-31" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["calendarEpisodes"].is_array());
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_unknown_field_returns_error() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ nonExistentField }", json!({})).await;
    assert!(
        body.get("errors").is_some(),
        "unknown field should return errors"
    );
}

#[tokio::test]
async fn graphql_invalid_mutation_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { addTitle(input: { name: "" }) { title { id } } }"#,
        json!({}),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "invalid input should return errors"
    );
}

#[tokio::test]
async fn graphql_batch_request_not_supported_via_single() {
    let ctx = TestContext::new().await;
    // Verify single requests work (batch is handled at the middleware level)
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
}

// ---------------------------------------------------------------------------
// Authentication flow
// ---------------------------------------------------------------------------

/// The login mutation is available without a pre-existing session.
/// After providing valid credentials, the server returns a non-empty JWT.
///
/// Note: the migration-seeded "admin" user has a NULL password_hash (it is
/// intended for dev-mode auto-login, not credential-based login).  We
/// therefore create a fresh user with an explicit password to exercise the
/// full login path.
#[tokio::test]
async fn login_with_valid_credentials_returns_token() {
    let ctx = TestContext::new().await;

    // Need an actor to create the test user — admin has all entitlements.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "logintest".to_string(),
            "s3cr3t!".to_string(),
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "logintest", password: "s3cr3t!" }) { token expiresAt user { username } } }"#,
        None,
    )
    .await;

    assert!(
        body["errors"].is_null(),
        "login should not return errors: {body}"
    );
    let token = body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty(), "JWT token should not be empty");
    assert_eq!(body["data"]["login"]["user"]["username"], "logintest");
}

/// Providing the wrong password must produce a GraphQL error — never a token.
#[tokio::test]
async fn login_with_wrong_password_returns_error() {
    let ctx = TestContext::new().await;

    // Create a user with a known password so we can test wrong-password rejection.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "wrongpasstest".to_string(),
            "correct_horse".to_string(),
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "wrongpasstest", password: "wrong_password" }) { token } }"#,
        None,
    )
    .await;

    assert!(
        !body["errors"].is_null()
            && body["errors"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
        "wrong password should return a GraphQL error: {body}"
    );
    // Verify the error indicates bad credentials, not a server error.
    let error_msg = body["errors"][0]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.to_ascii_lowercase().contains("credentials")
            || error_msg.to_ascii_lowercase().contains("invalid"),
        "error should indicate bad credentials: {error_msg}"
    );
}

/// Most queries require a user in the request context.  Executing one via the
/// schema directly (without injecting a User) must return an authentication
/// error rather than leaking data.
#[tokio::test]
async fn unauthenticated_request_returns_error() {
    let ctx = TestContext::new().await;

    // `titles` calls actor_from_ctx — must fail without a user in context.
    let body = schema_exec(&ctx, "{ titles { id } }", None).await;

    let errors = body["errors"].as_array().expect("should have errors");
    assert!(
        !errors.is_empty(),
        "unauthenticated request should return errors"
    );
    let messages: Vec<&str> = errors
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.to_ascii_lowercase().contains("auth")),
        "error message should mention authentication: {messages:?}"
    );
}

/// After obtaining a JWT via the login mutation, the caller can authenticate
/// that token to retrieve the User and use it on a protected query.
#[tokio::test]
async fn authenticated_request_with_valid_token_succeeds() {
    let ctx = TestContext::new().await;

    // Create a user with an explicit password and ViewCatalog so the
    // protected `titles` query can succeed.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "authtest".to_string(),
            "s3cr3t!".to_string(),
            vec![scryer_domain::Entitlement::ViewCatalog],
        )
        .await
        .unwrap();

    // Step 1: log in and capture the token.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "authtest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    // Step 2: validate the token to recover the User.
    let user = ctx
        .app
        .authenticate_token(&token)
        .await
        .expect("token should be valid");

    // Step 3: execute a protected query with the authenticated user attached.
    let body = schema_exec(&ctx, "{ titles { id } }", Some(user)).await;
    assert!(
        body["errors"].is_null(),
        "authenticated query should not error: {body}"
    );
    assert!(body["data"]["titles"].is_array());
}

#[tokio::test]
async fn token_is_revoked_after_set_user_entitlements_until_relogin() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let create_body = schema_exec(
        &ctx,
        r#"mutation {
            createUser(input: {
                username: "entrevoketest",
                password: "s3cr3t!",
                entitlements: ["view_catalog"]
            }) {
                id
                username
            }
        }"#,
        Some(admin.clone()),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    let user_id = create_body["data"]["createUser"]["id"]
        .as_str()
        .expect("created user id")
        .to_string();

    let login_before = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_before["errors"].is_null(),
        "initial login should succeed: {login_before}"
    );
    let old_token = login_before["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    let update_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                setUserEntitlements(input: {{
                    userId: "{user_id}",
                    entitlements: ["view_catalog", "manage_title"]
                }}) {{
                    id
                    entitlements
                }}
            }}"#
        ),
        Some(admin),
    )
    .await;
    assert!(
        update_body["errors"].is_null(),
        "setUserEntitlements should succeed: {update_body}"
    );

    let old_result = ctx.app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "token issued before entitlement change should be rejected"
    );

    let login_after = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_after["errors"].is_null(),
        "re-login should succeed after entitlement change: {login_after}"
    );
    let new_token = login_after["data"]["login"]["token"]
        .as_str()
        .expect("refreshed token should be a string")
        .to_string();

    let decoded = ctx
        .app
        .authenticate_token(&new_token)
        .await
        .expect("refreshed token should authenticate");
    assert!(
        decoded
            .entitlements
            .contains(&scryer_domain::Entitlement::ManageTitle),
        "re-issued token should carry updated entitlements"
    );
}

/// A token issued for a different issuer (or an arbitrary tampered token)
/// must be rejected by `authenticate_token` — not by a GraphQL error but as
/// a hard application-level failure.
#[tokio::test]
async fn tampered_token_is_rejected_by_authenticate_token() {
    let ctx = TestContext::new().await;

    // Craft a syntactically valid-looking but unsigned JWT (three base64 parts).
    let fake_token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJoYWNrZXIifQ.invalidsig";

    let result = ctx.app.authenticate_token(fake_token).await;
    assert!(
        result.is_err(),
        "tampered/unsigned token must not be accepted"
    );
}

/// Creating a user with `createUser` and then logging in as that user must
/// succeed end-to-end — confirming that the password is stored and validated
/// consistently.
#[tokio::test]
async fn newly_created_user_can_login() {
    let ctx = TestContext::new().await;

    // The admin user must exist before we can create another user
    // (createUser requires ManageConfig entitlement).
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    // Create a new user as admin.
    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "newuser", password: "s3cr3t!", entitlements: [] }) { id username } }"#,
        Some(admin),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    assert_eq!(create_body["data"]["createUser"]["username"], "newuser");

    // Log in as the newly created user.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "newuser", password: "s3cr3t!" }) { token user { username } } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "new user login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty());
    assert_eq!(login_body["data"]["login"]["user"]["username"], "newuser");
}
