mod backup;
mod collections;
mod config;
mod downloads;
mod external_import;
mod jobs;
mod library;
mod media_requests;
mod notifications;
mod plugins;
mod post_processing;
mod recycle_bin;
mod rules;
mod subtitle;
mod titles;
mod users;
mod wanted;

use async_graphql::MergedObject;
use scryer_interface_settings::SettingsMutations;

#[derive(MergedObject, Default)]
pub struct MutationRoot(
    titles::TitleMutations,
    collections::CollectionMutations,
    downloads::DownloadMutations,
    jobs::JobMutations,
    config::ConfigMutations,
    SettingsMutations,
    users::UserMutations,
    library::LibraryMutations,
    media_requests::MediaRequestMutations,
    wanted::WantedMutations,
    rules::RulesMutations,
    plugins::PluginMutations,
    notifications::NotificationMutations,
    backup::BackupMutations,
    external_import::ExternalImportMutations,
    post_processing::PostProcessingMutations,
    subtitle::SubtitleMutations,
    recycle_bin::RecycleBinMutations,
);
