use async_graphql::{ComplexObject, Context, ID, Result as GqlResult};
use scryer_application::{ReleaseDecisionsQuery, WantedItemsQuery};
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};

use crate::mappers::{
    from_collection, from_download_queue_item, from_episode, from_library_settings,
    from_pending_release, from_release_decision, from_series_movie_link, from_submission_scope,
    from_title, from_title_media_file, from_wanted_item,
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
    async fn required_audio_languages_override(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<Vec<String>>> {
        let app = app_from_ctx(ctx)?;
        app.load_title_required_audio_override(self.id.as_ref())
            .await
            .map_err(to_gql_error)
    }

    async fn effective_required_audio_languages(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<String>> {
        let app = app_from_ctx(ctx)?;
        if let Some(languages) = app
            .load_title_required_audio_override(self.id.as_ref())
            .await
            .map_err(to_gql_error)?
        {
            return Ok(languages);
        }
        app.load_facet_required_audio_languages(title_scope_from_facet(self.facet).as_scope_id())
            .await
            .map_err(to_gql_error)
    }

    async fn inherits_required_audio_languages(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        Ok(app
            .load_title_required_audio_override(self.id.as_ref())
            .await?
            .is_none())
    }

    async fn collections(&self, ctx: &Context<'_>) -> GqlResult<Vec<CollectionPayload>> {
        if let Some(collections) = &self.preloaded_collections {
            return Ok(collections.clone());
        }
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collections = app
            .list_collections(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(collections.into_iter().map(from_collection).collect())
    }

    async fn series_movie_links(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<SeriesMovieLinkPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let links = app
            .list_series_movie_links(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(links.into_iter().map(from_series_movie_link).collect())
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let files = app
            .list_title_media_files(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(files.into_iter().map(from_title_media_file).collect())
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<WantedStatusValue>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<WantedItemsPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = relation_page_limit(limit);
        let offset = relation_page_offset(offset);
        let (items, total_count) = app
            .list_wanted_items(
                &actor,
                WantedItemsQuery {
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
            limit,
            offset,
            has_more: i64::from(offset) + i64::from(limit) < total_count,
            total_count: total_count.min(i64::from(i32::MAX)) as i32,
        })
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
        let offset = relation_page_offset(offset);
        let decisions = app
            .list_release_decisions(
                &actor,
                ReleaseDecisionsQuery {
                    wanted_item_id: None,
                    title_id: Some(self.id.to_string()),
                    limit: i64::from(limit) + i64::from(offset) + 1,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let mut items = decisions
            .into_iter()
            .skip(offset as usize)
            .map(from_release_decision)
            .collect::<scryer_application::AppResult<Vec<_>>>()
            .map_err(to_gql_error)?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        Ok(ReleaseDecisionsPagePayload {
            items,
            limit,
            offset,
            has_more,
        })
    }

    async fn download_queue_items(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
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
    }
}

#[ComplexObject]
impl CollectionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episodes(&self, ctx: &Context<'_>) -> GqlResult<Vec<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_episodes(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_episode).collect())
    }
}

#[ComplexObject]
impl EpisodePayload {
    async fn parent_title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
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
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let files = app
            .list_title_media_files(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(files
            .into_iter()
            .filter(|file| file.episode_id.as_deref() == Some(self.id.as_ref()))
            .map(from_title_media_file)
            .collect())
    }
}

#[ComplexObject]
impl TitleMediaFilePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }
}

#[ComplexObject]
impl WantedItemPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
        let offset = relation_page_offset(offset);
        let decisions = app
            .list_release_decisions(
                &actor,
                ReleaseDecisionsQuery {
                    wanted_item_id: Some(self.id.to_string()),
                    title_id: None,
                    limit: i64::from(limit) + i64::from(offset) + 1,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let mut items = decisions
            .into_iter()
            .skip(offset as usize)
            .map(from_release_decision)
            .collect::<scryer_application::AppResult<Vec<_>>>()
            .map_err(to_gql_error)?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        Ok(ReleaseDecisionsPagePayload {
            items,
            limit,
            offset,
            has_more,
        })
    }

    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<PendingReleasesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = relation_page_limit(limit);
        let offset = relation_page_offset(offset);
        let releases = app
            .list_pending_releases_for_wanted_item(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        let total_count = releases.len().min(i32::MAX as usize) as i32;
        let items = releases
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(from_pending_release)
            .collect::<Vec<_>>();
        Ok(PendingReleasesPayload {
            items,
            limit,
            offset,
            has_more: i64::from(offset) + i64::from(limit) < i64::from(total_count),
            total_count,
        })
    }
}

#[ComplexObject]
impl ReleaseDecisionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
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
    }
}

#[ComplexObject]
impl DownloadQueueItemPayload {
    async fn queue_scope(&self, ctx: &Context<'_>) -> GqlResult<Option<QueueDownloadScopePayload>> {
        let client_type = self.client_type.trim();
        let download_client_item_id = self.download_client_item_id.trim();
        if client_type.is_empty() || download_client_item_id.is_empty() {
            return Ok(self
                .episode_id
                .as_ref()
                .map(|episode_id| QueueDownloadScopePayload {
                    kind: "episode".to_string(),
                    episode_id: Some(episode_id.clone().into()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                    series_movie_link_id: None,
                }));
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
                .map(|episode_id| QueueDownloadScopePayload {
                    kind: "episode".to_string(),
                    episode_id: Some(episode_id.clone().into()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                    series_movie_link_id: None,
                })
        }))
    }

    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let Some(title_id) = self.title_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title_for_management(&actor, title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }
}

#[ComplexObject]
impl PendingReleasePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title_for_management(&actor, self.title_id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
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
    }
}
