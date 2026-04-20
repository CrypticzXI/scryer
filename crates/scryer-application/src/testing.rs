use std::path::Path;

use crate::{AppServicesBuilder, AppUseCase};

pub async fn execute_upgrade_for_test(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    title: &scryer_domain::Title,
    existing_file: &crate::TitleMediaFile,
    source_path: &Path,
    dest_path: &Path,
    parsed: crate::ParsedReleaseMetadata,
    final_score: i32,
    target_episode_ids: &[String],
    recycle_config: &crate::recycle_bin::RecycleBinConfig,
) -> crate::AppResult<crate::upgrade::UpgradeResult> {
    let prepared = crate::post_download_gate::PreparedImportCandidate {
        parsed,
        accepted: Box::new(crate::post_download_gate::ImportedFileAcceptance {
            analysis: None,
            scan_error: None,
        }),
        rescore_changes: Vec::new(),
    };
    let old_score = existing_file.acquisition_score.unwrap_or(0);

    crate::upgrade::execute_upgrade(
        app,
        actor,
        title,
        existing_file,
        source_path,
        dest_path,
        &prepared,
        final_score,
        old_score,
        target_episode_ids,
        recycle_config,
    )
    .await
}

pub trait AppUseCaseTestExt {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder;

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64>;
}

impl AppUseCaseTestExt for AppUseCase {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        AppUseCase::with_test_overrides(self, configure)
    }

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64> {
        self.runtime.events.notification_event_broadcast.subscribe()
    }
}
