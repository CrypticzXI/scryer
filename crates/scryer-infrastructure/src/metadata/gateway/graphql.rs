pub(crate) const SEARCH_TVDB_QUERY: &str = include_str!("metadata_gateway/search_tvdb.graphql");
pub(crate) const SEARCH_TVDB_BATCH_QUERY: &str =
    include_str!("metadata_gateway/search_tvdb_batch.graphql");
pub(crate) const SEARCH_TVDB_RICH_QUERY: &str =
    include_str!("metadata_gateway/search_tvdb_rich.graphql");
pub(crate) const SEARCH_TVDB_MULTI_QUERY: &str =
    include_str!("metadata_gateway/search_tvdb_multi.graphql");
pub(crate) const GET_MOVIE_QUERY: &str = include_str!("metadata_gateway/get_movie.graphql");
pub(crate) const GET_SERIES_QUERY: &str = include_str!("metadata_gateway/get_series.graphql");
pub(crate) const DISCOVER_PUBLIC_FEED_QUERY: &str =
    include_str!("metadata_gateway/discover_public_feed.graphql");
pub(crate) const TITLE_RECOMMENDATIONS_QUERY: &str =
    include_str!("metadata_gateway/title_recommendations.graphql");
pub(crate) const COLLECTION_COMPLETIONS_QUERY: &str =
    include_str!("metadata_gateway/collection_completions.graphql");
pub(crate) const SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT_QUERY: &str =
    include_str!("metadata_gateway/submit_discovery_context_snapshot.graphql");
pub(crate) const DISCOVERY_CONTEXT_SNAPSHOT_STATUS_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_snapshot_status.graphql");
pub(crate) const DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_snapshot_page.graphql");
pub(crate) const DISCOVERY_CONTEXT_CHANGES_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_changes.graphql");
pub(crate) const ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT_QUERY: &str =
    include_str!("metadata_gateway/acknowledge_discovery_context_snapshot.graphql");
pub(crate) const MOVIE_FIELDS_FRAGMENT: &str =
    include_str!("metadata_gateway/movie_fields.graphql");
pub(crate) const SERIES_FIELDS_FRAGMENT: &str =
    include_str!("metadata_gateway/series_fields.graphql");
