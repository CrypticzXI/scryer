use std::path::Path;

use crate::{AppServicesBuilder, AppUseCase};

pub struct UpgradeForTestInput<'a> {
    pub actor: &'a scryer_domain::User,
    pub title: &'a scryer_domain::Title,
    pub existing_file: &'a crate::TitleMediaFile,
    pub source_path: &'a Path,
    pub dest_path: &'a Path,
    pub parsed: crate::ParsedReleaseMetadata,
    pub final_score: i32,
    pub target_episode_ids: &'a [String],
    pub recycle_config: &'a crate::recycle_bin::RecycleBinConfig,
}

pub async fn execute_upgrade_for_test(
    app: &AppUseCase,
    input: UpgradeForTestInput<'_>,
) -> crate::AppResult<crate::upgrade::UpgradeResult> {
    let UpgradeForTestInput {
        actor,
        title,
        existing_file,
        source_path,
        dest_path,
        parsed,
        final_score,
        target_episode_ids,
        recycle_config,
    } = input;
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
        prepared.parsed.quality.as_deref(),
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
