use async_graphql::{ComplexObject, Context, ID, Result as GqlResult};
use scryer_application::{AcquisitionScopeStatesQuery, ReleaseDecisionsQuery};
use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, loaders::loaders_from_ctx, to_gql_error,
};

use crate::mappers::{
    from_collection, from_discovery_item, from_download_queue_item, from_episode,
    from_library_settings, from_pending_release, from_release_decision, from_series_movie_link,
    from_submission_scope, from_title, from_title_media_file, from_title_rating_summary,
    from_wanted_item,
};
use crate::types::*;

const RELATION_PAGE_MAX_LIMIT: i32 = 300;

fn title_scope_from_facet(facet: MediaFacetValue) -> ContentScopeValue {
    match facet {
        MediaFacetValue::Movie => ContentScopeValue::Movie,
        MediaFacetValue::Series => ContentScopeValue::Series,
        MediaFacetValue::Anime => ContentScopeValue::Anime,
    }
}

fn relation_page_limit(limit: i32) -> i32 {
    limit.clamp(1, RELATION_PAGE_MAX_LIMIT)
}

fn relation_page_offset(offset: i32) -> i32 {
    offset.max(0)
}

#[ComplexObject]
impl LibraryPayload {
    async fn quality_profile_id(&self, ctx: &Context<'_>) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.title_quality_profile_id_for_library(&actor, self.id.as_ref())
            .await
            .map(Into::into)
            .map_err(to_gql_error)
    }

    async fn request_quality_profile_ids(&self, ctx: &Context<'_>) -> GqlResult<Vec<ID>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .request_quality_profile_settings_for_library(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(settings.profile_ids.into_iter().map(Into::into).collect())
    }

    async fn request_quality_profile_default_id(&self, ctx: &Context<'_>) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .request_quality_profile_settings_for_library(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(settings.default_profile_id.into())
    }

    async fn settings(&self, ctx: &Context<'_>) -> GqlResult<LibrarySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_library_settings(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_settings(settings))
    }
}

#[ComplexObject]
impl TitlePayload {
    /// Legacy title quality label.
    async fn quality_tier(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .primary_collection_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.label));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_primary_collection_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.label))
        })
        .await
    }

    /// Lowest live media-file quality tier for the title.
    async fn current_quality_tier(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .quality_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.quality_tier));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_quality_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.quality_tier))
        })
        .await
    }

    /// Aggregated media-file size in bytes for the title.
    async fn size_bytes(&self, ctx: &Context<'_>) -> GqlResult<Option<Long>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .media_size_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| Long::from(summary.total_size_bytes)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_media_size_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| Long::from(summary.total_size_bytes)))
        })
        .await
    }

    /// Owned-vs-total episode progress, excluding specials.
    async fn episodes_owned(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.owned_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.owned_episodes))
        })
        .await
    }

    /// Monitored episode count, excluding specials.
    async fn episodes_monitored(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.monitored_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.monitored_episodes))
        })
        .await
    }

    /// Total episode count, excluding specials.
    async fn episodes_total(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.total_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.total_episodes))
        })
        .await
    }

    /// Primary movie media resolution.
    async fn media_resolution(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.resolution));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.resolution))
        })
        .await
    }

    /// Primary movie media HDR format.
    async fn media_hdr(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.hdr_format));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.hdr_format))
        })
        .await
    }

    /// Primary movie media audio codec.
    async fn media_audio_codec(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.audio_codec));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.audio_codec))
        })
        .await
    }

    async fn library_name(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let library = loaders
                .library
                .load_one(self.library_id.to_string())
                .await?;
            return Ok(library.map(|library| library.name));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let library_id = self.library_id.to_string();
            let libraries = app
                .list_libraries_for_permission(&actor, None, scryer_domain::LibraryPermission::View)
                .await
                .map_err(to_gql_error)?;
            Ok(libraries
                .into_iter()
                .find(|library| library.id == library_id)
                .map(|library| library.name))
        })
        .await
    }

    async fn library_slug(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let library = loaders
                .library
                .load_one(self.library_id.to_string())
                .await?;
            return Ok(library.map(|library| library.slug));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let library_id = self.library_id.to_string();
            let libraries = app
                .list_libraries_for_permission(&actor, None, scryer_domain::LibraryPermission::View)
                .await
                .map_err(to_gql_error)?;
            Ok(libraries
                .into_iter()
                .find(|library| library.id == library_id)
                .map(|library| library.slug))
        })
        .await
    }

    async fn ratings(&self, ctx: &Context<'_>) -> GqlResult<TitleRatingPayload> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders.ratings.load_one(self.id.to_string()).await?;
            return Ok(summary.map(from_title_rating_summary).unwrap_or_else(|| {
                TitleRatingPayload {
                    rating: None,
                    rating_sources: Vec::new(),
                    external_ratings: Vec::new(),
                }
            }));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            app.title_ratings(&actor, self.id.as_ref())
                .await
                .map(from_title_rating_summary)
                .map_err(to_gql_error)
        })
        .await
    }

    async fn more_like_this(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 12)] limit: i32,
    ) -> GqlResult<Vec<DiscoveryItemPayload>> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let items = app
                .title_more_like_this(&actor, self.id.as_ref(), i64::from(limit.clamp(0, 100)))
                .await
                .map_err(to_gql_error)?;
            Ok(items.into_iter().map(from_discovery_item).collect())
        })
        .await
    }

    async fn root_folder_path(&self, ctx: &Context<'_>) -> GqlResult<String> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            app.require_library_permission(
                &actor,
                self.library_id.as_ref(),
                scryer_domain::LibraryPermission::View,
            )
            .await
            .map_err(to_gql_error)?;
            app.title_root_folder_path_for_parts(
                self.root_folder_id.as_ref(),
                self.library_id.as_ref(),
                &self.facet.into_domain(),
            )
            .await
            .map_err(to_gql_error)
        })
        .await
    }

    async fn required_audio_languages_override(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<Vec<String>>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return loaders
                .required_audio_override
                .load_one(self.id.to_string())
                .await;
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            app.load_title_required_audio_override(self.id.as_ref())
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    async fn effective_required_audio_languages(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<String>> {
        Box::pin(async move {
            let override_languages = if let Some(loaders) = loaders_from_ctx(ctx) {
                loaders
                    .required_audio_override
                    .load_one(self.id.to_string())
                    .await?
            } else {
                app_from_ctx(ctx)?
                    .load_title_required_audio_override(self.id.as_ref())
                    .await
                    .map_err(to_gql_error)?
            };
            if let Some(languages) = override_languages {
                return Ok(languages);
            }
            app_from_ctx(ctx)?
                .load_facet_required_audio_languages(
                    title_scope_from_facet(self.facet).as_scope_id(),
                )
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    async fn inherits_required_audio_languages(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return Ok(loaders
                .required_audio_override
                .load_one(self.id.to_string())
                .await?
                .is_none());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            Ok(app
                .load_title_required_audio_override(self.id.as_ref())
                .await?
                .is_none())
        })
        .await
    }

    async fn collections(&self, ctx: &Context<'_>) -> GqlResult<Vec<CollectionPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collections = loaders
                .collections_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(collections.into_iter().map(from_collection).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collections = app
                .list_collections(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(collections.into_iter().map(from_collection).collect())
        })
        .await
    }

    async fn series_movie_links(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<SeriesMovieLinkPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let links = loaders
                .series_movie_links_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(links.into_iter().map(from_series_movie_link).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let links = app
                .list_series_movie_links(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(links.into_iter().map(from_series_movie_link).collect())
        })
        .await
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let files = loaders
                .media_files_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(files.into_iter().map(from_title_media_file).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let files = app
                .list_title_media_files(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(files.into_iter().map(from_title_media_file).collect())
        })
        .await
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<WantedStatusValue>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<WantedItemsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit);
            let offset = relation_page_offset(offset);
            let (items, _total_count) = app
                .list_acquisition_scope_states(
                    &actor,
                    AcquisitionScopeStatesQuery {
                        statuses: status
                            .map(|value| value.as_str().to_string())
                            .into_iter()
                            .collect(),
                        media_types: Vec::new(),
                        title_id: Some(self.id.to_string()),
                        library_ids: Vec::new(),
                        title_search: None,
                        latest_decision_codes: Vec::new(),
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            Ok(WantedItemsPagePayload {
                items: items
                    .into_iter()
                    .map(from_wanted_item)
                    .collect::<scryer_application::AppResult<Vec<_>>>()
                    .map_err(to_gql_error)?,
            })
        })
        .await
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
            let offset = relation_page_offset(offset);
            let (decisions, total_count) = app
                .list_release_decisions_page(
                    &actor,
                    ReleaseDecisionsQuery {
                        wanted_item_id: None,
                        title_id: Some(self.id.to_string()),
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            let items = decisions
                .into_iter()
                .map(from_release_decision)
                .collect::<scryer_application::AppResult<Vec<_>>>()
                .map_err(to_gql_error)?;
            let has_more = i64::from(offset).saturating_add(items.len() as i64) < total_count;
            Ok(ReleaseDecisionsPagePayload {
                items,
                total_count,
                has_more,
            })
        })
        .await
    }

    async fn download_queue_items(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let items = app
                .list_download_queue_for_title(
                    &actor,
                    self.id.as_ref(),
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
                .map_err(to_gql_error)?;
            Ok(items.into_iter().map(from_download_queue_item).collect())
        })
        .await
    }
}

#[ComplexObject]
impl CollectionPayload {
    /// Byte size of the file backing this collection, resolved on demand from the
    /// media-file index (`media_files.file_path` matching this collection's
    /// `ordered_path`). Resolved lazily so reads that omit the field pay nothing.
    async fn file_size_bytes(&self, ctx: &Context<'_>) -> GqlResult<Option<Long>> {
        let Some(ordered_path) = self.ordered_path.clone() else {
            return Ok(None);
        };
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let size = app
                .collection_media_size_bytes(&actor, self.title_id.as_ref(), &ordered_path)
                .await
                .map_err(to_gql_error)?;
            Ok(size.map(Long::from))
        })
        .await
    }

    /// Owned-vs-total episode progress for this collection, populated when requested.
    async fn episodes_owned(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.owned_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.owned_episodes))
        })
        .await
    }

    /// Monitored episode count for this collection, populated when requested.
    async fn episodes_monitored(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.monitored_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.monitored_episodes))
        })
        .await
    }

    /// Total countable episode count for this collection, populated when requested.
    async fn episodes_total(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.total_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.total_episodes))
        })
        .await
    }

    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn episodes(&self, ctx: &Context<'_>) -> GqlResult<Vec<EpisodePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episodes = loaders
                .episodes_for_collection
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(episodes.into_iter().map(from_episode).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episodes = app
                .list_episodes(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(episodes.into_iter().map(from_episode).collect())
        })
        .await
    }
}

#[ComplexObject]
impl EpisodePayload {
    async fn parent_title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collection = loaders
                .collection
                .load_one(collection_id.to_string())
                .await?;
            return Ok(collection.map(from_collection));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collection = app
                .get_collection(&actor, collection_id)
                .await
                .map_err(to_gql_error)?
                .map(from_collection);
            Ok(collection)
        })
        .await
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let state = loaders
                .title_wanted_item
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return state
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let wanted_item = app
                .get_title_wanted_item(&actor, self.title_id.as_ref(), Some(self.id.as_ref()))
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(wanted_item)
        })
        .await
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let files = loaders
                .episode_media_files
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?
                .unwrap_or_default();
            return Ok(files.into_iter().map(from_title_media_file).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let files = app
                .list_episode_media_files(&actor, self.title_id.as_ref(), self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(files.into_iter().map(from_title_media_file).collect())
        })
        .await
    }
}

#[ComplexObject]
impl TitleMediaFilePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episode = loaders.episode.load_one(episode_id.to_string()).await?;
            return Ok(episode.map(from_episode));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episode = app
                .get_episode(&actor, episode_id)
                .await
                .map_err(to_gql_error)?
                .map(from_episode);
            Ok(episode)
        })
        .await
    }
}

#[ComplexObject]
impl WantedItemPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collection = loaders
                .collection
                .load_one(collection_id.to_string())
                .await?;
            return Ok(collection.map(from_collection));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collection = app
                .get_collection(&actor, collection_id)
                .await
                .map_err(to_gql_error)?
                .map(from_collection);
            Ok(collection)
        })
        .await
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episode = loaders.episode.load_one(episode_id.to_string()).await?;
            return Ok(episode.map(from_episode));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episode = app
                .get_episode(&actor, episode_id)
                .await
                .map_err(to_gql_error)?
                .map(from_episode);
            Ok(episode)
        })
        .await
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
            let offset = relation_page_offset(offset);
            let (decisions, total_count) = app
                .list_release_decisions_page(
                    &actor,
                    ReleaseDecisionsQuery {
                        wanted_item_id: Some(self.id.to_string()),
                        title_id: None,
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            let items = decisions
                .into_iter()
                .map(from_release_decision)
                .collect::<scryer_application::AppResult<Vec<_>>>()
                .map_err(to_gql_error)?;
            let has_more = i64::from(offset).saturating_add(items.len() as i64) < total_count;
            Ok(ReleaseDecisionsPagePayload {
                items,
                total_count,
                has_more,
            })
        })
        .await
    }

    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<PendingReleasesPayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit);
            let offset = relation_page_offset(offset);
            let (releases, total) = app
                .list_pending_releases_for_wanted_item_page(
                    &actor,
                    self.id.as_ref(),
                    i64::from(limit),
                    i64::from(offset),
                )
                .await
                .map_err(to_gql_error)?;
            let total_count = total.min(i64::from(i32::MAX)) as i32;
            let items = releases
                .into_iter()
                .map(from_pending_release)
                .collect::<Vec<_>>();
            let has_more =
                i64::from(offset).saturating_add(items.len() as i64) < i64::from(total_count);
            Ok(PendingReleasesPayload {
                items,
                has_more,
                total_count,
            })
        })
        .await
    }
}

#[ComplexObject]
impl ReleaseDecisionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let item = loaders
                .wanted_item
                .load_one(self.wanted_item_id.to_string())
                .await?;
            return item.map(from_wanted_item).transpose().map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let item = app
                .get_wanted_item(&actor, self.wanted_item_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(item)
        })
        .await
    }
}

#[ComplexObject]
impl DownloadQueueItemPayload {
    async fn queue_scope(&self, ctx: &Context<'_>) -> GqlResult<Option<QueueDownloadScopePayload>> {
        Box::pin(async move {
            let client_type = self.client_type.trim();
            let download_client_item_id = self.download_client_item_id.trim();
            if client_type.is_empty() || download_client_item_id.is_empty() {
                return Ok(self
                    .episode_id
                    .as_ref()
                    .map(|episode_id| QueueDownloadScopePayload::episode(episode_id.clone())));
            }

            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let client_id = self.client_id.trim();
            let client_id = if client_id.is_empty() {
                None
            } else {
                Some(client_id)
            };
            let scope = app
                .find_download_queue_scope(&actor, client_id, client_type, download_client_item_id)
                .await
                .map_err(to_gql_error)?;

            Ok(scope.map(from_submission_scope).or_else(|| {
                self.episode_id
                    .as_ref()
                    .map(|episode_id| QueueDownloadScopePayload::episode(episode_id.clone()))
            }))
        })
        .await
    }

    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let Some(title_id) = self.title_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders
                .title_for_management
                .load_one(title_id.to_string())
                .await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title_for_management(&actor, title_id)
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }
}

#[ComplexObject]
impl PendingReleasePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders
                .title_for_management
                .load_one(self.title_id.to_string())
                .await?;
            return Ok(title.map(from_title));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title_for_management(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_title);
            Ok(title)
        })
        .await
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let item = loaders
                .wanted_item_for_management
                .load_one(self.wanted_item_id.to_string())
                .await?;
            return item.map(from_wanted_item).transpose().map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let wanted_item = app
                .get_wanted_item_for_management(&actor, self.wanted_item_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(wanted_item)
        })
        .await
    }
}
