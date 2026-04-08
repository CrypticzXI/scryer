use async_trait::async_trait;
use scryer_domain::MediaFacet;

use crate::facet_handler::{FacetHandler, HydrationResult, movie_to_hydration_result};
use crate::{ActivityKind, AppResult, MetadataGateway};

pub struct MovieFacetHandler;

#[async_trait]
impl FacetHandler for MovieFacetHandler {
    fn facet(&self) -> MediaFacet {
        MediaFacet::Movie
    }

    fn facet_id(&self) -> &str {
        "movie"
    }

    fn download_category(&self) -> &str {
        "movie"
    }

    fn library_path_key(&self) -> &str {
        "movies.path"
    }

    fn root_folders_key(&self) -> &str {
        "movies.root_folders"
    }

    fn default_library_path(&self) -> &str {
        "/data/movies"
    }

    fn has_episodes(&self) -> bool {
        false
    }

    fn title_added_activity_kind(&self) -> Option<ActivityKind> {
        Some(ActivityKind::MovieAdded)
    }

    fn search_category(&self) -> &str {
        "movie"
    }

    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult> {
        let movie = gateway.get_movie(tvdb_id, language).await?;
        Ok(movie_to_hydration_result(movie, language))
    }
}
