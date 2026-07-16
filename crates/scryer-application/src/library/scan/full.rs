use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "movie library scans are top-level orchestration entry points with explicit runtime state"
)]
pub(super) async fn scan_library_movies(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<LibraryScanSummary> {
    run_library_scan_pipeline(LibraryScanPipelineRequest {
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        mark_discovery_complete_on_drain,
        cancel_token,
        scan_hints: scan_hints.cloned(),
        kind: LibraryScanPipelineKind::Movie,
    })
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "series library scans are top-level orchestration entry points with explicit runtime state"
)]
pub(super) async fn scan_library_series(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<LibraryScanSummary> {
    run_library_scan_pipeline(LibraryScanPipelineRequest {
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        mark_discovery_complete_on_drain,
        cancel_token,
        scan_hints: scan_hints.cloned(),
        kind: LibraryScanPipelineKind::Series,
    })
    .await
}
