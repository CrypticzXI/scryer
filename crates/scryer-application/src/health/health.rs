use super::*;
use std::collections::HashSet;
use tracing::warn;

#[cfg(unix)]
fn to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

fn health_root_label(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "Movies",
        MediaFacet::Series => "Series",
        MediaFacet::Anime => "Anime",
    }
}

impl AppUseCase {
    /// Run all health checks and return results.
    pub async fn run_health_checks(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        results.extend(self.check_download_clients().await);
        results.extend(self.check_indexers().await);
        results.extend(self.check_root_folders().await);
        results.extend(self.check_disk_space_health().await);
        results
    }

    async fn check_download_clients(&self) -> Vec<HealthCheckResult> {
        let configs = match self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "health check: failed to list download clients");
                return vec![HealthCheckResult {
                    source: "DownloadClient".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to query download clients: {e}"),
                }];
            }
        };

        if configs.is_empty() {
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Error,
                message: "No download client is configured".into(),
            }];
        }

        let enabled: Vec<_> = configs.iter().filter(|c| c.is_enabled).collect();
        if enabled.is_empty() {
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: "All download clients are disabled".into(),
            }];
        }

        let errored: Vec<_> = enabled
            .iter()
            .filter(|c| {
                c.status == scryer_domain::DownloadClientStatus::Error
                    || c.status == scryer_domain::DownloadClientStatus::Failed
            })
            .collect();
        if !errored.is_empty() {
            let names: Vec<&str> = errored.iter().map(|c| c.name.as_str()).collect();
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: format!("Download client(s) reporting errors: {}", names.join(", ")),
            }];
        }

        let mut results = Vec::new();
        for config in enabled {
            let has_remote_path_mappings =
                match crate::has_download_client_remote_path_mappings(&config.config_json) {
                    Ok(value) => value,
                    Err(error) => {
                        results.push(HealthCheckResult {
                            source: "DownloadClient".into(),
                            status: HealthCheckStatus::Warning,
                            message: format!(
                                "Download client '{}' has invalid remote path mappings: {error}",
                                config.name
                            ),
                        });
                        continue;
                    }
                };

            let status = match self
                .services
                .integrations
                .download_client
                .get_client_status_for_client_id(&config.id)
                .await
            {
                Ok(status) => status,
                Err(_) => continue,
            };

            let unresolved_roots = status
                .remote_output_roots
                .iter()
                .map(|path| path.trim())
                .filter(|path| !path.is_empty())
                .filter(|path| !std::path::Path::new(path).exists())
                .map(ToString::to_string)
                .collect::<Vec<_>>();

            if unresolved_roots.is_empty() {
                continue;
            }

            if status.is_localhost != Some(false) && !has_remote_path_mappings {
                continue;
            }

            results.push(HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: format!(
                    "Download client '{}' reports output paths that Scryer still cannot access after remote path mapping: {}. Check remote path mappings and container volume mounts.",
                    config.name,
                    unresolved_roots.join(", ")
                ),
            });
        }

        results
    }

    async fn check_indexers(&self) -> Vec<HealthCheckResult> {
        let configs = match self.services.integrations.indexer_configs.list(None).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "health check: failed to list indexers");
                return vec![HealthCheckResult {
                    source: "Indexer".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to query indexers: {e}"),
                }];
            }
        };

        if configs.is_empty() {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Warning,
                message: "No indexer is configured".into(),
            }];
        }

        let enabled: Vec<_> = configs.iter().filter(|c| c.is_enabled).collect();
        if enabled.is_empty() {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Warning,
                message: "All indexers are disabled".into(),
            }];
        }

        let stats = self.services.integrations.indexer_stats.all_stats();
        let all_failing = !stats.is_empty()
            && stats
                .iter()
                .all(|s| s.failed_last_24h > 0 && s.successful_last_24h == 0);
        if all_failing {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Error,
                message: "All indexers are failing".into(),
            }];
        }

        vec![]
    }

    async fn check_root_folders(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let label = health_root_label(&facet);
            let root_folders = match self.root_folders_for_facet(&facet).await {
                Ok(root_folders) => root_folders,
                Err(error) => {
                    results.push(HealthCheckResult {
                        source: "RootFolder".into(),
                        status: HealthCheckStatus::Error,
                        message: format!("Failed to resolve {label} roots: {error}"),
                    });
                    continue;
                }
            };

            for root in root_folders {
                let path = root.path.trim();
                if path.is_empty() {
                    continue;
                }
                let p = std::path::Path::new(path);
                if !p.exists() {
                    results.push(HealthCheckResult {
                        source: "RootFolder".into(),
                        status: HealthCheckStatus::Error,
                        message: format!("{label} root folder does not exist: {path}"),
                    });
                } else if p
                    .metadata()
                    .map(|m| m.permissions().readonly())
                    .unwrap_or(true)
                {
                    results.push(HealthCheckResult {
                        source: "RootFolder".into(),
                        status: HealthCheckStatus::Warning,
                        message: format!("{label} root folder is read-only: {path}"),
                    });
                }
            }
        }

        results
    }

    async fn check_disk_space_health(&self) -> Vec<HealthCheckResult> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let label = health_root_label(&facet);
            let root_folders = match self.root_folders_for_facet(&facet).await {
                Ok(root_folders) => root_folders,
                Err(error) => {
                    results.push(HealthCheckResult {
                        source: "DiskSpace".into(),
                        status: HealthCheckStatus::Error,
                        message: format!("Failed to resolve {label} roots: {error}"),
                    });
                    continue;
                }
            };

            for root in root_folders {
                let path = root.path.trim();
                if path.is_empty() || !seen.insert(path.to_string()) {
                    continue;
                }

                #[cfg(unix)]
                if let Some(stat) = statvfs_path(path) {
                    let free = to_u64(stat.f_bavail) * to_u64(stat.f_frsize);
                    let mb_100 = 100 * 1024 * 1024;
                    let mb_500 = 500 * 1024 * 1024;

                    if free < mb_100 {
                        results.push(HealthCheckResult {
                            source: "DiskSpace".into(),
                            status: HealthCheckStatus::Error,
                            message: format!(
                                "{label} disk space critically low: {} MB free at {path}",
                                free / (1024 * 1024)
                            ),
                        });
                    } else if free < mb_500 {
                        results.push(HealthCheckResult {
                            source: "DiskSpace".into(),
                            status: HealthCheckStatus::Warning,
                            message: format!(
                                "{label} disk space low: {} MB free at {path}",
                                free / (1024 * 1024)
                            ),
                        });
                    }
                }
            }
        }

        results
    }
}
