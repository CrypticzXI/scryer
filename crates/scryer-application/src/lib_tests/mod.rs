use super::*;
use async_trait::async_trait;
use base64::Engine as _;
use scryer_domain::{
    Collection, CollectionType, DomainEventFilter, DomainEventPayload, DomainEventType, Episode,
    EpisodeType, EventType, ImportSkipReason, ImportType, JobRunCompletedEventData,
    JobRunStartedEventData, MediaRequestRequester, MediaRequestStatus, RootFolderEntry,
    TrackedDownloadState,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, Instant, sleep, timeout};

mod acquisition_recovery;
mod discovery_sync;
mod downloads;
mod libraries;
mod library_scan;
mod media_requests;
mod queueing;
mod routing_settings;
mod search_cutoff;
mod security_auth;
mod series_metadata;
mod title_hydration;
mod title_image_cache;
mod title_updates;
mod user_permissions;
mod users_admin_titles;

mod support_acquisition_downloads;
mod support_bootstrap_fixtures;
mod support_catalog;
mod support_events_requests;
mod support_imports;
mod support_indexers_metadata;
mod support_library_show;
mod support_settings_scan;
use support_acquisition_downloads::*;
pub(crate) use support_bootstrap_fixtures::bootstrap;
use support_bootstrap_fixtures::*;
use support_catalog::*;
use support_events_requests::*;
use support_imports::*;
use support_indexers_metadata::*;
use support_library_show::*;
use support_settings_scan::*;
