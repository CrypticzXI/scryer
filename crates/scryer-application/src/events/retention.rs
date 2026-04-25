use scryer_domain::DomainEventType;

pub(crate) const OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DomainEventRetentionClass {
    UserFacingHistory,
    OperationalProjection,
}

pub(crate) const fn retention_class_for_domain_event_type(
    event_type: DomainEventType,
) -> DomainEventRetentionClass {
    match event_type {
        DomainEventType::TitleAdded
        | DomainEventType::TitleUpdated
        | DomainEventType::TitleRematched
        | DomainEventType::TitleDeleted
        | DomainEventType::ConfigurationChanged
        | DomainEventType::DiscoverySearchCompleted
        | DomainEventType::MetadataHydrationUpdated
        | DomainEventType::ReleaseGrabbed
        | DomainEventType::DownloadFailed
        | DomainEventType::ImportCompleted
        | DomainEventType::ImportRejected
        | DomainEventType::MediaFileImported
        | DomainEventType::MediaFileAnalyzed
        | DomainEventType::MediaFileRenamed
        | DomainEventType::MediaFileDeleted
        | DomainEventType::MediaFileUpgraded
        | DomainEventType::AcquisitionSearchCompleted
        | DomainEventType::AcquisitionCandidateRejected
        | DomainEventType::ImportRequested
        | DomainEventType::ImportRecoveryCompleted
        | DomainEventType::DownloadQueueItemCommandIssued
        | DomainEventType::PostProcessingCompleted
        | DomainEventType::SubtitleDownloaded
        | DomainEventType::SubtitleSearchFailed => DomainEventRetentionClass::UserFacingHistory,
        DomainEventType::LibraryScanStarted
        | DomainEventType::LibraryScanTitleDiscovered
        | DomainEventType::LibraryScanDeltaRecorded
        | DomainEventType::LibraryScanProgressed
        | DomainEventType::LibraryScanCompleted
        | DomainEventType::LibraryScanCanceled
        | DomainEventType::LibraryScanFailed
        | DomainEventType::JobRunStarted
        | DomainEventType::JobRunCompleted
        | DomainEventType::JobRunFailed
        | DomainEventType::JobNextRunUpdated
        | DomainEventType::DownloadQueueItemUpserted
        | DomainEventType::DownloadQueueItemRemoved => {
            DomainEventRetentionClass::OperationalProjection
        }
    }
}

pub(crate) fn domain_event_types_for_class(
    class: DomainEventRetentionClass,
) -> Vec<DomainEventType> {
    DomainEventType::variants()
        .filter(|event_type| retention_class_for_domain_event_type(*event_type) == class)
        .collect()
}

pub(crate) fn user_facing_domain_event_types() -> Vec<DomainEventType> {
    domain_event_types_for_class(DomainEventRetentionClass::UserFacingHistory)
}

pub(crate) fn operational_domain_event_types() -> Vec<DomainEventType> {
    domain_event_types_for_class(DomainEventRetentionClass::OperationalProjection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn domain_event_retention_classification_covers_known_types_without_duplicates() {
        let mut seen = HashSet::new();
        for event_type in DomainEventType::variants() {
            assert!(
                seen.insert(event_type.as_str()),
                "duplicate domain event type in retention classification: {}",
                event_type.as_str()
            );
        }

        assert_eq!(
            seen.len(),
            user_facing_domain_event_types().len() + operational_domain_event_types().len()
        );

        for event_type in user_facing_domain_event_types() {
            assert_eq!(
                retention_class_for_domain_event_type(event_type),
                DomainEventRetentionClass::UserFacingHistory
            );
        }

        for event_type in operational_domain_event_types() {
            assert_eq!(
                retention_class_for_domain_event_type(event_type),
                DomainEventRetentionClass::OperationalProjection
            );
        }
    }
}
