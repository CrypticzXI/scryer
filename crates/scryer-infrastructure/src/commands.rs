use crate::queries::{
    blocklist as blocklist_queries, library_scan_unmatched as library_scan_unmatched_queries,
    title::*, title_history as th_queries,
};
use scryer_application::{
    AppResult, LibraryScanUnmatchedItem, PendingRelease, PendingReleaseStatus, ReleaseDecision,
    WantedItem,
};
use scryer_domain::{BlocklistEntry, MediaFacet, TitleHistoryRecord};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use tokio::sync::oneshot::Sender;

pub(crate) enum DbCommand {
    UpsertLibraryScanUnmatchedItem {
        item: LibraryScanUnmatchedItem,
        reply: Sender<AppResult<String>>,
    },
    DeleteLibraryScanUnmatchedItem {
        facet: MediaFacet,
        item_path: String,
        reply: Sender<AppResult<()>>,
    },
    ListLibraryScanUnmatchedItems {
        facet: Option<MediaFacet>,
        scan_root: Option<String>,
        limit: i64,
        offset: i64,
        reply: Sender<AppResult<Vec<LibraryScanUnmatchedItem>>>,
    },
    CountLibraryScanUnmatchedItems {
        facet: Option<MediaFacet>,
        scan_root: Option<String>,
        reply: Sender<AppResult<i64>>,
    },
    ListEpisodesForTitle {
        title_id: String,
        reply: Sender<AppResult<Vec<scryer_domain::Episode>>>,
    },
    FindEpisodeByTitleAndNumbers {
        title_id: String,
        season_number: String,
        episode_number: String,
        reply: Sender<AppResult<Option<scryer_domain::Episode>>>,
    },
    FindEpisodeByTitleAndAbsoluteNumber {
        title_id: String,
        absolute_number: String,
        reply: Sender<AppResult<Option<scryer_domain::Episode>>>,
    },
    UpsertWantedItem {
        item: WantedItem,
        reply: Sender<AppResult<String>>,
    },
    EnsureWantedItemSeeded {
        item: WantedItem,
        reply: Sender<AppResult<String>>,
    },
    ListDueWantedItems {
        now: String,
        batch_limit: i64,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    UpdateWantedItemStatus {
        id: String,
        status: String,
        next_search_at: Option<String>,
        last_search_at: Option<String>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    GetWantedItemForTitle {
        title_id: String,
        episode_id: Option<String>,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    DeleteWantedItemsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteWantedItemsForCollection {
        collection_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteWantedItemsForEpisode {
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    ResetFruitlessWantedItems {
        now: String,
        reply: Sender<AppResult<u64>>,
    },
    InsertReleaseDecision {
        decision: ReleaseDecision,
        reply: Sender<AppResult<String>>,
    },
    GetWantedItemById {
        id: String,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    ListWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    CountWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        reply: Sender<AppResult<i64>>,
    },
    ListReleaseDecisionsForTitle {
        title_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
    ListReleaseDecisionsForWantedItem {
        wanted_item_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
    // ── Pending Releases ──────────────────────────────────────────────
    InsertPendingRelease {
        release: PendingRelease,
        reply: Sender<AppResult<String>>,
    },
    ListExpiredPendingReleases {
        now: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    ListPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    UpdatePendingReleaseStatus {
        id: String,
        status: PendingReleaseStatus,
        grabbed_at: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    ListStandbyPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    DeleteStandbyPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListAllStandbyPendingReleases {
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    CompareAndSetPendingReleaseStatus {
        id: String,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<String>,
        reply: Sender<AppResult<bool>>,
    },
    SupersedePendingReleasesForWantedItem {
        wanted_item_id: String,
        except_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListWaitingPendingReleases {
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    GetPendingRelease {
        id: String,
        reply: Sender<AppResult<Option<PendingRelease>>>,
    },
    DeletePendingReleasesForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Title History ─────────────────────────────────────────────────
    InsertTitleHistoryEvent {
        title_id: String,
        episode_id: Option<String>,
        collection_id: Option<String>,
        event_type: String,
        source_title: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListTitleHistory {
        event_types: Option<Vec<String>>,
        title_ids: Option<Vec<String>>,
        download_id: Option<String>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForTitle {
        title_id: String,
        event_types: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForEpisode {
        episode_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    FindTitleHistoryByDownloadId {
        download_id: String,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    DeleteTitleHistoryForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Blocklist ─────────────────────────────────────────────────────
    InsertBlocklistEntry {
        title_id: String,
        source_title: Option<String>,
        source_hint: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        reason: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListBlocklistForTitle {
        title_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<BlocklistEntry>>>,
    },
    ListBlocklistAll {
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<BlocklistEntry>, i64)>>,
    },
    DeleteBlocklistEntry {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    IsBlocklisted {
        title_id: String,
        source_title: String,
        reply: Sender<AppResult<bool>>,
    },
    DeleteBlocklistForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
}

pub(crate) fn spawn_db_command_worker(pool: SqlitePool) -> mpsc::Sender<DbCommand> {
    let (sender, mut receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                DbCommand::UpsertLibraryScanUnmatchedItem { item, reply } => {
                    let _ = reply.send(
                        library_scan_unmatched_queries::upsert_library_scan_unmatched_item_query(
                            &pool, &item,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteLibraryScanUnmatchedItem {
                    facet,
                    item_path,
                    reply,
                } => {
                    let _ = reply.send(
                        library_scan_unmatched_queries::delete_library_scan_unmatched_item_query(
                            &pool, facet, &item_path,
                        )
                        .await,
                    );
                }
                DbCommand::ListLibraryScanUnmatchedItems {
                    facet,
                    scan_root,
                    limit,
                    offset,
                    reply,
                } => {
                    let _ = reply.send(
                        library_scan_unmatched_queries::list_library_scan_unmatched_items_query(
                            &pool,
                            facet,
                            scan_root.as_deref(),
                            limit,
                            offset,
                        )
                        .await,
                    );
                }
                DbCommand::CountLibraryScanUnmatchedItems {
                    facet,
                    scan_root,
                    reply,
                } => {
                    let _ = reply.send(
                        library_scan_unmatched_queries::count_library_scan_unmatched_items_query(
                            &pool,
                            facet,
                            scan_root.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListEpisodesForTitle { title_id, reply } => {
                    let _ = reply.send(list_episodes_for_title_query(&pool, &title_id).await);
                }
                DbCommand::FindEpisodeByTitleAndNumbers {
                    title_id,
                    season_number,
                    episode_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_numbers_query(
                            &pool,
                            &title_id,
                            &season_number,
                            &episode_number,
                        )
                        .await,
                    );
                }
                DbCommand::FindEpisodeByTitleAndAbsoluteNumber {
                    title_id,
                    absolute_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_absolute_number_query(
                            &pool,
                            &title_id,
                            &absolute_number,
                        )
                        .await,
                    );
                }
                DbCommand::UpsertWantedItem { item, reply } => {
                    let _ = reply
                        .send(crate::queries::wanted::upsert_wanted_item_query(&pool, &item).await);
                }
                DbCommand::EnsureWantedItemSeeded { item, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::ensure_wanted_item_seeded_query(&pool, &item).await,
                    );
                }
                DbCommand::ListDueWantedItems {
                    now,
                    batch_limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_due_wanted_items_query(
                            &pool,
                            &now,
                            batch_limit,
                        )
                        .await,
                    );
                }
                DbCommand::UpdateWantedItemStatus {
                    id,
                    status,
                    next_search_at,
                    last_search_at,
                    search_count,
                    current_score,
                    grabbed_release,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::update_wanted_item_status_query(
                            &pool,
                            &id,
                            &status,
                            next_search_at.as_deref(),
                            last_search_at.as_deref(),
                            search_count,
                            current_score,
                            grabbed_release.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetWantedItemForTitle {
                    title_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_for_title_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForCollection {
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_collection_query(
                            &pool,
                            &collection_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForEpisode { episode_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_episode_query(
                            &pool,
                            &episode_id,
                        )
                        .await,
                    );
                }
                DbCommand::ResetFruitlessWantedItems { now, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::reset_fruitless_wanted_items_query(&pool, &now)
                            .await,
                    );
                }
                DbCommand::InsertReleaseDecision { decision, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::insert_release_decision_query(&pool, &decision)
                            .await,
                    );
                }
                DbCommand::GetWantedItemById { id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_by_id_query(&pool, &id).await,
                    );
                }
                DbCommand::ListWantedItems {
                    status,
                    media_type,
                    title_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                            limit,
                            offset,
                        )
                        .await,
                    );
                }
                DbCommand::CountWantedItems {
                    status,
                    media_type,
                    title_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::count_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_title_query(
                            &pool, &title_id, limit,
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForWantedItem {
                    wanted_item_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                            limit,
                        )
                        .await,
                    );
                }
                // ── Pending Releases ──────────────────────────────────────
                DbCommand::InsertPendingRelease { release, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::insert_pending_release_query(
                            &pool, &release,
                        )
                        .await,
                    );
                }
                DbCommand::ListExpiredPendingReleases { now, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_expired_pending_releases_query(
                            &pool, &now,
                        )
                        .await,
                    );
                }
                DbCommand::ListPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id,
                        ).await,
                    );
                }
                DbCommand::UpdatePendingReleaseStatus {
                    id,
                    status,
                    grabbed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::update_pending_release_status_query(
                            &pool,
                            &id,
                            status,
                            grabbed_at.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListStandbyPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_standby_pending_releases_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteStandbyPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::delete_standby_pending_releases_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                        )
                        .await,
                    );
                }
                DbCommand::ListAllStandbyPendingReleases { reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_all_standby_pending_releases_query(
                            &pool,
                        )
                        .await,
                    );
                }
                DbCommand::CompareAndSetPendingReleaseStatus {
                    id,
                    current_status,
                    next_status,
                    grabbed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::compare_and_set_pending_release_status_query(
                            &pool,
                            &id,
                            current_status,
                            next_status,
                            grabbed_at.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::SupersedePendingReleasesForWantedItem {
                    wanted_item_id,
                    except_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::supersede_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id, &except_id,
                        ).await,
                    );
                }
                DbCommand::ListWaitingPendingReleases { reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_waiting_pending_releases_query(
                            &pool,
                        )
                        .await,
                    );
                }
                DbCommand::GetPendingRelease { id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::get_pending_release_query(&pool, &id)
                            .await,
                    );
                }
                DbCommand::DeletePendingReleasesForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::delete_pending_releases_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                // ── Title History ─────────────────────────────────────────
                DbCommand::InsertTitleHistoryEvent {
                    title_id,
                    episode_id,
                    collection_id,
                    event_type,
                    source_title,
                    quality,
                    download_id,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        th_queries::insert_title_history_event_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                            collection_id.as_deref(),
                            &event_type,
                            source_title.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListTitleHistory {
                    event_types,
                    title_ids,
                    download_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_query(
                        &pool,
                        type_strs.as_deref(),
                        title_ids.as_deref(),
                        download_id.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForTitle {
                    title_id,
                    event_types,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_for_title_query(
                        &pool,
                        &title_id,
                        type_strs.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForEpisode {
                    episode_id,
                    limit,
                    reply,
                } => {
                    let res =
                        th_queries::list_title_history_for_episode_query(&pool, &episode_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::FindTitleHistoryByDownloadId { download_id, reply } => {
                    let res =
                        th_queries::find_title_history_by_download_id_query(&pool, &download_id)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::DeleteTitleHistoryForTitle { title_id, reply } => {
                    let _ = reply.send(
                        th_queries::delete_title_history_for_title_query(&pool, &title_id).await,
                    );
                }
                // ── Blocklist ─────────────────────────────────────────────
                DbCommand::InsertBlocklistEntry {
                    title_id,
                    source_title,
                    source_hint,
                    quality,
                    download_id,
                    reason,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::insert_blocklist_entry_query(
                            &pool,
                            &title_id,
                            source_title.as_deref(),
                            source_hint.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            reason.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListBlocklistForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let res =
                        blocklist_queries::list_blocklist_for_title_query(&pool, &title_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(bl_row_to_entry).collect());
                    let _ = reply.send(res);
                }
                DbCommand::ListBlocklistAll {
                    limit,
                    offset,
                    reply,
                } => {
                    let res = blocklist_queries::list_blocklist_all_query(&pool, limit, offset)
                        .await
                        .map(|(rows, total)| {
                            let entries = rows.into_iter().map(bl_row_to_entry).collect();
                            (entries, total)
                        });
                    let _ = reply.send(res);
                }
                DbCommand::DeleteBlocklistEntry { id, reply } => {
                    let _ = reply
                        .send(blocklist_queries::delete_blocklist_entry_query(&pool, &id).await);
                }
                DbCommand::IsBlocklisted {
                    title_id,
                    source_title,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::is_blocklisted_query(&pool, &title_id, &source_title)
                            .await,
                    );
                }
                DbCommand::DeleteBlocklistForTitle { title_id, reply } => {
                    let _ = reply.send(
                        blocklist_queries::delete_blocklist_for_title_query(&pool, &title_id).await,
                    );
                }
            }
        }
    });

    sender
}

fn th_row_to_record(row: th_queries::TitleHistoryRow) -> TitleHistoryRecord {
    TitleHistoryRecord {
        id: row.id,
        title_id: row.title_id,
        episode_id: row.episode_id,
        collection_id: row.collection_id,
        event_type: scryer_domain::TitleHistoryEventType::parse(&row.event_type)
            .unwrap_or(scryer_domain::TitleHistoryEventType::Grabbed),
        source_title: row.source_title,
        quality: row.quality,
        download_id: row.download_id,
        data_json: row.data_json,
        occurred_at: row.occurred_at,
        created_at: row.created_at,
    }
}

fn bl_row_to_entry(row: blocklist_queries::BlocklistRow) -> BlocklistEntry {
    BlocklistEntry {
        id: row.id,
        title_id: row.title_id,
        source_title: row.source_title,
        source_hint: row.source_hint,
        quality: row.quality,
        download_id: row.download_id,
        reason: row.reason,
        data_json: row.data_json,
        created_at: row.created_at,
    }
}
