#[cfg(test)]
mod tests {
    use super::{
        DownloadQueueBucket, apply_tracked_download_queue_metadata, classify_download_queue_item,
        dedupe_download_queue_items, derive_download_queue_display_state,
        tracked_download_queue_snapshot,
    };
    use crate::DownloadDisplayState;
    use chrono::Utc;
    use scryer_domain::{
        DownloadQueueItem, DownloadQueueState, ImportStatus, TitleMatchType, TrackedDownloadState,
        TrackedDownloadStatus,
    };

    fn item(id: &str, state: DownloadQueueState) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent: 100,
            size_bytes: Some(100),
            remaining_seconds: None,
            queued_at: Some(Utc::now().timestamp_millis().to_string()),
            last_updated_at: Some(Utc::now().timestamp_millis().to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    #[test]
    fn dedupe_download_queue_items_merges_duplicate_client_job_ids() {
        let mut first = item("job-1", DownloadQueueState::Completed);
        first.import_error_message = Some("failed to import".to_string());
        let mut second = item("job-1", DownloadQueueState::Completed);
        second.title_id = Some("title-1".to_string());

        let deduped = dedupe_download_queue_items(vec![
            first,
            second,
            item("job-2", DownloadQueueState::Queued),
        ]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].download_client_item_id, "job-1");
        assert_eq!(deduped[0].title_id.as_deref(), Some("title-1"));
        assert_eq!(
            deduped[0].import_error_message.as_deref(),
            Some("failed to import")
        );
    }

    #[test]
    fn dedupe_download_queue_items_keeps_same_native_id_from_different_clients() {
        let mut first = item("job-1", DownloadQueueState::Queued);
        first.client_id = "client-1".to_string();
        let mut second = item("job-1", DownloadQueueState::Queued);
        second.client_id = "client-2".to_string();

        let deduped = dedupe_download_queue_items(vec![first, second]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].client_id, "client-1");
        assert_eq!(deduped[1].client_id, "client-2");
    }

    #[test]
    fn apply_tracked_download_queue_metadata_backfills_missing_facet() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            state: TrackedDownloadState::ImportBlocked,
            status: TrackedDownloadStatus::Warning,
            status_messages: vec!["needs manual import".to_string()],
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
            skip_reacquire_on_failure: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(queue_item.title_id.as_deref(), Some("title-1"));
        assert_eq!(queue_item.facet.as_deref(), Some("series"));
        assert_eq!(
            queue_item.tracked_state,
            Some(TrackedDownloadState::ImportBlocked)
        );
        assert_eq!(
            queue_item.tracked_status,
            Some(TrackedDownloadStatus::Warning)
        );
        assert_eq!(
            queue_item.tracked_match_type,
            Some(TitleMatchType::TitleParse)
        );
    }

    #[test]
    fn failed_source_state_stays_out_of_import_bucket() {
        let mut queue_item = item("job-failed", DownloadQueueState::Failed);
        queue_item.import_status = Some(ImportStatus::Failed);
        queue_item.tracked_state = Some(TrackedDownloadState::ImportBlocked);
        queue_item.import_error_message = Some("manual import failed".to_string());

        let classified = classify_download_queue_item(&queue_item);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Failed
        );
        assert_eq!(classified.bucket, DownloadQueueBucket::HistoryFailed);
    }

    #[test]
    fn apply_tracked_download_queue_metadata_prefers_source_release_title() {
        let mut queue_item = item("job-1", DownloadQueueState::Downloading);
        queue_item.title_name = "Titanic".to_string();
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: Some("Titanic.1997.2160p.UHD.BluRay.x265-GRP".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
            skip_reacquire_on_failure: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(
            queue_item.title_name,
            "Titanic.1997.2160p.UHD.BluRay.x265-GRP"
        );
    }
}
