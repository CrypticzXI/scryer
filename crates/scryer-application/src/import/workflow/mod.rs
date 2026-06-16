use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AppError, AppResult, AppUseCase, DownloadSourceIdentity, DownloadSubmission,
    DownloadSubmissionIdentity, ImportArtifact, ParsedEpisodeMetadata, ParsedReleaseMetadata,
    SubmissionScope, WantedCompleteTransition, WantedItemsQuery,
    activity::NotificationMediaUpdate,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    apply_remote_path_mappings_to_completed_download,
    domain_events::{
        created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
    },
    effective_title_folder_path,
    helpers::{
        has_usable_release_title_signal, normalize_release_title_signal, parse_usable_release_title,
    },
    import_parameters::{extract_parameter, has_scryer_origin, submission_has_scryer_origin},
    import_title_resolution::normalize_imdb_id,
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_download_client_remote_path_mappings, parse_release_metadata,
    polling_worker::PollingWorker,
    render_rename_template, sanitize_filesystem_component,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DomainEventPayload, DownloadQueueItem,
    DownloadQueueState, Id, ImportCompletedEventData, ImportDecision, ImportErrorCode,
    ImportRecord, ImportResult, ImportSkipReason, ImportStatus, ImportType, MediaFacet, Title,
    TrackedDownloadState, User, is_video_file,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("poller.rs");
include!("completed.rs");
include!("movie.rs");
include!("series_movie.rs");
include!("series.rs");
include!("paths.rs");
include!("metadata.rs");
include!("wanted.rs");
include!("manual.rs");
include!("results.rs");
include!("tests.rs");
