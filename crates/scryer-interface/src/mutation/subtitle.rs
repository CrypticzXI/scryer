use async_graphql::{Context, InputObject, Object, SimpleObject};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};

#[derive(InputObject)]
pub struct DeleteExternalSubtitleInput {
    pub external_subtitle_id: String,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
pub struct BlocklistExternalSubtitleInput {
    pub external_subtitle_id: String,
    pub reason: Option<String>,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

type GqlResult<T> = async_graphql::Result<T>;

#[derive(Default)]
pub struct SubtitleMutations;

#[derive(InputObject)]
pub struct SearchSubtitlesInput {
    pub media_file_id: String,
    pub language: String,
}

#[derive(InputObject)]
pub struct DownloadSubtitleInput {
    pub media_file_id: String,
    pub provider: Option<String>,
    pub provider_file_id: String,
    pub language: String,
    pub forced: Option<bool>,
    pub hearing_impaired: Option<bool>,
    pub score: Option<i32>,
    pub release_info: Option<String>,
    pub uploader: Option<String>,
    pub ai_translated: Option<bool>,
    pub machine_translated: Option<bool>,
}

#[derive(SimpleObject)]
pub struct SubtitleSearchResult {
    pub provider: String,
    pub provider_file_id: String,
    pub language: String,
    pub release_info: Option<String>,
    pub score: i32,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub ai_translated: bool,
    pub machine_translated: bool,
    pub uploader: Option<String>,
    pub download_count: Option<i64>,
    pub hash_matched: bool,
}

fn from_subtitle_match(
    result: scryer_application::subtitles::SubtitleMatch,
) -> SubtitleSearchResult {
    SubtitleSearchResult {
        provider: result.provider,
        provider_file_id: result.provider_file_id,
        language: result.language,
        release_info: result.release_info,
        score: result.score,
        hearing_impaired: result.hearing_impaired,
        forced: result.forced,
        ai_translated: result.ai_translated,
        machine_translated: result.machine_translated,
        uploader: result.uploader,
        download_count: result.download_count,
        hash_matched: result.hash_matched,
    }
}

#[Object]
impl SubtitleMutations {
    /// Search for subtitles for a media file in a given language.
    async fn search_subtitles(
        &self,
        ctx: &Context<'_>,
        input: SearchSubtitlesInput,
    ) -> GqlResult<Vec<SubtitleSearchResult>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let results = app
            .search_subtitles_for_media_file(&actor, &input.media_file_id, &input.language)
            .await
            .map_err(to_gql_error)?;
        Ok(results.into_iter().map(from_subtitle_match).collect())
    }

    /// Download a specific subtitle and save to disk next to the video.
    async fn download_subtitle(
        &self,
        ctx: &Context<'_>,
        input: DownloadSubtitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.download_subtitle_for_media_file(
            &actor,
            &input.media_file_id,
            input.provider.as_deref().unwrap_or("opensubtitles"),
            &input.provider_file_id,
            &input.language,
            input.forced.unwrap_or(false),
            input.hearing_impaired.unwrap_or(false),
            input.score,
            input.release_info,
            input.uploader,
            input.ai_translated.unwrap_or(false),
            input.machine_translated.unwrap_or(false),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Delete an external subtitle file and its tracked record.
    async fn delete_external_subtitle(
        &self,
        ctx: &Context<'_>,
        input: DeleteExternalSubtitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            async_graphql::Error::new(
                "delete preview confirmation is required before deleting subtitle files on disk",
            )
        })?;

        app.delete_external_subtitle(
            &actor,
            &input.external_subtitle_id,
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(true)
    }

    /// Blocklist a downloaded provider-backed subtitle: delete the file and DB record, then add to the blocklist.
    async fn blocklist_external_subtitle(
        &self,
        ctx: &Context<'_>,
        input: BlocklistExternalSubtitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            async_graphql::Error::new(
                "delete preview confirmation is required before deleting subtitle files on disk",
            )
        })?;

        app.blocklist_external_subtitle(
            &actor,
            &input.external_subtitle_id,
            input.reason.as_deref(),
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(true)
    }
}
