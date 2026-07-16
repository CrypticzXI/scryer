use super::*;
use crate::domain_events::new_download_queue_domain_event;
use crate::event_views::{
    apply_download_queue_projection_event, sort_download_queue_items, sorted_download_queue_items,
};
use crate::tracked_downloads::{
    TrackedDownload, TrackedDownloadQueueMetadata, publish_runtime_tracked_download_snapshot_cache,
    tracked_download_id_for_item,
};
use crate::types::DownloadClientFilterOption;
use scryer_domain::{
    CompletedDownload, DomainEventFilter, DomainEventPayload, DomainEventType,
    DownloadQueueDeleteStatus, DownloadQueueItemRemovedEventData,
    DownloadQueueItemUpsertedEventData, ImportType, TrackedDownloadState, TrackedDownloadStatus,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("indexers.rs");
include!("indexer_proxies.rs");
include!("managed_indexers.rs");
include!("download_clients.rs");
include!("queue_projection.rs");
include!("queue_queries.rs");
include!("manual_import_sources.rs");
include!("tracked_commands.rs");
include!("queue_mutations.rs");
include!("subscriptions.rs");
include!("permissions.rs");
include!("tests.rs");
