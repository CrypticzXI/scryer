use async_graphql::{Context, ID, InputObject, Object, SimpleObject};
use scryer_application::{AppError, DownloadSubtitleForMediaFileRequest};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};

#[derive(InputObject)]
pub struct DeleteExternalSubtitleInput {
    pub external_subtitle_id: ID,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
pub struct BlocklistExternalSubtitleInput {
    pub external_subtitle_id: ID,
    pub reason: Option<String>,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

type GqlResult<T> = async_graphql::Result<T>;

#[derive(Default)]
pub struct SubtitleMutations;

#[derive(InputObject)]
pub struct SearchSubtitlesInput {
    pub media_file_id: ID,
    pub language: String,
}

#[derive(InputObject)]
pub struct DownloadSubtitleInput {
    pub media_file_id: ID,
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
pub struct DownloadSubtitlePayload {
    pub media_file_id: ID,
    pub provider_file_id: String,
    pub downloaded: bool,
}

#[derive(SimpleObject)]
pub struct DeleteExternalSubtitlePayload {
    pub id: ID,
    pub deleted: bool,
}

#[derive(SimpleObject)]
pub struct BlocklistExternalSubtitlePayload {
    pub id: ID,
    pub blocklisted: bool,
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
        let media_file_id = input.media_file_id.to_string();
        let results = app
            .search_subtitles_for_media_file(&actor, &media_file_id, &input.language)
            .await
            .map_err(to_gql_error)?;
        Ok(results.into_iter().map(from_subtitle_match).collect())
    }

    /// Download a specific subtitle and save to disk next to the video.
    async fn download_subtitle(
        &self,
        ctx: &Context<'_>,
        input: DownloadSubtitleInput,
    ) -> GqlResult<DownloadSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let media_file_id = input.media_file_id;
        let media_file_id_string = media_file_id.to_string();
        let provider_file_id = input.provider_file_id;
        app.download_subtitle_for_media_file(
            &actor,
            DownloadSubtitleForMediaFileRequest {
                media_file_id: media_file_id_string,
                provider_name: input
                    .provider
                    .unwrap_or_else(|| "opensubtitles".to_string()),
                provider_file_id: provider_file_id.clone(),
                language: input.language,
                forced: input.forced.unwrap_or(false),
                hearing_impaired: input.hearing_impaired.unwrap_or(false),
                score: input.score,
                release_info: input.release_info,
                uploader: input.uploader,
                ai_translated: input.ai_translated.unwrap_or(false),
                machine_translated: input.machine_translated.unwrap_or(false),
            },
        )
        .await
        .map_err(to_gql_error)?;
        Ok(DownloadSubtitlePayload {
            media_file_id,
            provider_file_id,
            downloaded: true,
        })
    }

    /// Delete an external subtitle file and its tracked record.
    async fn delete_external_subtitle(
        &self,
        ctx: &Context<'_>,
        input: DeleteExternalSubtitleInput,
    ) -> GqlResult<DeleteExternalSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = input.external_subtitle_id;
        let external_subtitle_id_string = external_subtitle_id.to_string();
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            to_gql_error(AppError::Validation(
                "delete preview confirmation is required before deleting subtitle files on disk"
                    .to_string(),
            ))
        })?;

        app.delete_external_subtitle(
            &actor,
            &external_subtitle_id_string,
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(DeleteExternalSubtitlePayload {
            id: external_subtitle_id,
            deleted: true,
        })
    }

    /// Blocklist a downloaded provider-backed subtitle: delete the file and DB record, then add to the blocklist.
    async fn blocklist_external_subtitle(
        &self,
        ctx: &Context<'_>,
        input: BlocklistExternalSubtitleInput,
    ) -> GqlResult<BlocklistExternalSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = input.external_subtitle_id;
        let external_subtitle_id_string = external_subtitle_id.to_string();
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            to_gql_error(AppError::Validation(
                "delete preview confirmation is required before deleting subtitle files on disk"
                    .to_string(),
            ))
        })?;

        app.blocklist_external_subtitle(
            &actor,
            &external_subtitle_id_string,
            input.reason.as_deref(),
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(BlocklistExternalSubtitlePayload {
            id: external_subtitle_id,
            blocklisted: true,
        })
    }
}
