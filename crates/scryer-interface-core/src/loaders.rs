//! Request-scoped dataloaders for relationship and enrichment fields.
//!
//! Freshness contract: a [`RequestLoaders`] instance lives for exactly one
//! GraphQL HTTP request — it is built in the axum handler after the actor is
//! resolved and injected via `request.data(...)`. The dataloader cache is
//! therefore a per-request consistency snapshot, never a cross-request cache;
//! it needs no TTL and no invalidation because it dies with the request.
//!
//! The WebSocket subscription path deliberately gets NO loaders: WS data is
//! per-connection, and a loader there would cache for the socket lifetime.
//! Resolvers must treat the absence of `RequestLoaders` in context as normal
//! and fall back to direct application calls.
//!
//! Permission semantics: every batch application method is actor-scoped and
//! silently drops ids the actor cannot see, so a missing key in a loader
//! result means "absent or not visible" — exactly the `None`/empty semantics
//! the nullable relationship fields already have.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_graphql::dataloader::{DataLoader, Loader};
use scryer_application::{
    AppUseCase, CollectionEpisodeProgressSummary, PrimaryCollectionSummary,
    TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary, TitleMovieMediaSummary,
    TitleQualitySummary, TitleRatingSummary,
};
use scryer_domain::{Collection, Episode, Library, Title, User};

use crate::to_gql_error;

type GqlError = async_graphql::Error;

/// Shared per-request context captured by every loader.
struct LoaderCtx {
    app: AppUseCase,
    actor: User,
}

/// Coalescing window for batching key lookups within one request.
const LOAD_DELAY: Duration = Duration::from_millis(1);

macro_rules! loader {
    ($name:ident, $key:ty, $value:ty, |$ctx:ident, $keys:ident| $body:expr) => {
        pub struct $name(Arc<LoaderCtx>);

        impl Loader<$key> for $name {
            type Value = $value;
            type Error = GqlError;

            async fn load(
                &self,
                $keys: &[$key],
            ) -> Result<HashMap<$key, Self::Value>, Self::Error> {
                let $ctx = &*self.0;
                $body
            }
        }
    };
}

/// Index a `Vec<T>` into a map keyed by an id extractor.
fn by_id<K, T>(items: Vec<T>, key: impl Fn(&T) -> K) -> HashMap<K, T>
where
    K: std::hash::Hash + Eq,
{
    items.into_iter().map(|item| (key(&item), item)).collect()
}

loader!(TitleLoader, String, Title, |ctx, keys| {
    let titles = ctx
        .app
        .get_titles_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(titles, |t| t.id.clone()))
});

loader!(TitleForManagementLoader, String, Title, |ctx, keys| {
    let titles = ctx
        .app
        .get_titles_by_ids_for_management(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(titles, |t| t.id.clone()))
});

loader!(LibraryLoader, String, Library, |ctx, keys| {
    // One permission-scoped listing covers every key; libraries are few.
    let libraries = ctx
        .app
        .list_libraries_for_permission(&ctx.actor, None, scryer_domain::LibraryPermission::View)
        .await
        .map_err(to_gql_error)?;
    let mut map = by_id(libraries, |l| l.id.clone());
    map.retain(|id, _| keys.contains(id));
    Ok(map)
});

loader!(CollectionLoader, String, Collection, |ctx, keys| {
    let collections = ctx
        .app
        .get_collections_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(collections, |c| c.id.clone()))
});

loader!(EpisodeLoader, String, Episode, |ctx, keys| {
    let episodes = ctx
        .app
        .get_episodes_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(episodes, |e| e.id.clone()))
});

loader!(
    MediaFilesForTitleLoader,
    String,
    Vec<TitleMediaFile>,
    |ctx, keys| {
        ctx.app
            .list_media_files_for_titles(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    CollectionsForTitleLoader,
    String,
    Vec<Collection>,
    |ctx, keys| {
        // Two batched calls: resolve visible titles, then their collections.
        let titles = ctx
            .app
            .get_titles_by_ids(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        ctx.app
            .list_collections_for_titles(&ctx.actor, &titles)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    EpisodesForCollectionLoader,
    String,
    Vec<Episode>,
    |ctx, keys| {
        ctx.app
            .list_episodes_for_collections(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    RequiredAudioOverrideLoader,
    String,
    Vec<String>,
    |ctx, keys| {
        ctx.app
            .load_title_required_audio_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    PrimaryCollectionSummaryLoader,
    String,
    PrimaryCollectionSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_primary_collection_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    MediaSizeSummaryLoader,
    String,
    TitleMediaSizeSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_media_size_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    QualitySummaryLoader,
    String,
    TitleQualitySummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_quality_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    EpisodeProgressSummaryLoader,
    String,
    TitleEpisodeProgressSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_episode_progress_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    CollectionEpisodeProgressLoader,
    String,
    Vec<CollectionEpisodeProgressSummary>,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_collection_episode_progress_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        let mut map: HashMap<String, Vec<CollectionEpisodeProgressSummary>> = HashMap::new();
        for summary in summaries {
            map.entry(summary.collection_id.clone()).or_default().push(summary);
        }
        Ok(map)
    }
);

loader!(
    RatingsLoader,
    String,
    TitleRatingSummary,
    |ctx, keys| {
        let ratings = ctx
            .app
            .list_title_ratings(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(ratings.into_iter().collect())
    }
);

loader!(
    MovieMediaSummaryLoader,
    String,
    TitleMovieMediaSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_movie_media_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

/// Everything a request needs to resolve relationships with batched reads.
///
/// Build once per HTTP request; never store beyond it.
pub struct RequestLoaders {
    pub title: DataLoader<TitleLoader>,
    pub title_for_management: DataLoader<TitleForManagementLoader>,
    pub library: DataLoader<LibraryLoader>,
    pub collection: DataLoader<CollectionLoader>,
    pub episode: DataLoader<EpisodeLoader>,
    pub media_files_for_title: DataLoader<MediaFilesForTitleLoader>,
    pub collections_for_title: DataLoader<CollectionsForTitleLoader>,
    pub episodes_for_collection: DataLoader<EpisodesForCollectionLoader>,
    pub required_audio_override: DataLoader<RequiredAudioOverrideLoader>,
    pub primary_collection_summary: DataLoader<PrimaryCollectionSummaryLoader>,
    pub media_size_summary: DataLoader<MediaSizeSummaryLoader>,
    pub quality_summary: DataLoader<QualitySummaryLoader>,
    pub episode_progress_summary: DataLoader<EpisodeProgressSummaryLoader>,
    pub collection_episode_progress: DataLoader<CollectionEpisodeProgressLoader>,
    pub ratings: DataLoader<RatingsLoader>,
    pub movie_media_summary: DataLoader<MovieMediaSummaryLoader>,
}

impl RequestLoaders {
    pub fn new(app: AppUseCase, actor: User) -> Arc<Self> {
        let ctx = Arc::new(LoaderCtx { app, actor });
        macro_rules! dl {
            ($loader:ident) => {
                DataLoader::new($loader(ctx.clone()), tokio::spawn).delay(LOAD_DELAY)
            };
        }
        Arc::new(Self {
            title: dl!(TitleLoader),
            title_for_management: dl!(TitleForManagementLoader),
            library: dl!(LibraryLoader),
            collection: dl!(CollectionLoader),
            episode: dl!(EpisodeLoader),
            media_files_for_title: dl!(MediaFilesForTitleLoader),
            collections_for_title: dl!(CollectionsForTitleLoader),
            episodes_for_collection: dl!(EpisodesForCollectionLoader),
            required_audio_override: dl!(RequiredAudioOverrideLoader),
            primary_collection_summary: dl!(PrimaryCollectionSummaryLoader),
            media_size_summary: dl!(MediaSizeSummaryLoader),
            quality_summary: dl!(QualitySummaryLoader),
            episode_progress_summary: dl!(EpisodeProgressSummaryLoader),
            collection_episode_progress: dl!(CollectionEpisodeProgressLoader),
            ratings: dl!(RatingsLoader),
            movie_media_summary: dl!(MovieMediaSummaryLoader),
        })
    }
}

/// Fetch the request's loaders, if this execution path injected them.
///
/// `None` on the WebSocket/subscription path (and any other path that does
/// not inject loaders) — callers must fall back to direct application calls.
pub fn loaders_from_ctx<'a>(
    ctx: &'a async_graphql::Context<'_>,
) -> Option<&'a Arc<RequestLoaders>> {
    ctx.data_opt::<Arc<RequestLoaders>>()
}
