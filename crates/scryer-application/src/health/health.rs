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

fn normalize_health_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(windows)]
    {
        trimmed
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }

    #[cfg(not(windows))]
    {
        trimmed.replace('\\', "/").trim_end_matches('/').to_string()
    }
}

fn path_overlaps(left: &str, right: &str) -> bool {
    let left = normalize_health_path(left);
    let right = normalize_health_path(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }

    left == right
        || left
            .strip_prefix(&right)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('\\'))
        || right
            .strip_prefix(&left)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('\\'))
}

fn download_client_status_health_results(
    config_name: &str,
    status: &DownloadClientStatus,
    has_remote_path_mappings: bool,
    library_roots: &[String],
) -> Vec<HealthCheckResult> {
    let unresolved_roots = status
        .remote_output_roots
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .filter(|path| !std::path::Path::new(path).exists())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let overlapping_roots = status
        .remote_output_roots
        .iter()
        .filter_map(|download_root| {
            library_roots
                .iter()
                .find(|library_root| path_overlaps(download_root, library_root))
                .map(|library_root| {
                    format!("{} overlaps {}", download_root.trim(), library_root.trim())
                })
        })
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    if !unresolved_roots.is_empty()
        && (status.is_localhost == Some(false) || has_remote_path_mappings)
    {
        results.push(HealthCheckResult {
            source: "DownloadClient".into(),
            status: HealthCheckStatus::Warning,
            message: format!(
                "Download client '{}' reports output paths that Scryer still cannot access after remote path mapping: {}. Check remote path mappings and container volume mounts.",
                config_name,
                unresolved_roots.join(", ")
            ),
        });
    }

    if !overlapping_roots.is_empty() {
        results.push(HealthCheckResult {
            source: "DownloadClient".into(),
            status: HealthCheckStatus::Warning,
            message: format!(
                "Download client '{}' reports output roots that overlap library roots: {}. Separate download and library folders to avoid blocked completed-download imports.",
                config_name,
                overlapping_roots.join(", ")
            ),
        });
    }

    results
}

impl AppUseCase {
    /// Run all health checks and return results.
    pub async fn run_health_checks(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        results.extend(self.check_download_clients().await);
        results.extend(self.check_indexers().await);
        results.extend(self.check_root_folders().await);
        results.extend(self.check_recycle_bin_config().await);
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

        let mut library_roots = Vec::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let roots = match self.root_folders_for_facet(&facet).await {
                Ok(roots) => roots,
                Err(error) => {
                    warn!(
                        error = %error,
                        facet = ?facet,
                        "health check: failed to resolve library roots while checking download clients"
                    );
                    Vec::new()
                }
            };

            for root in roots {
                let path = root.path.trim();
                if !path.is_empty() {
                    library_roots.push(path.to_string());
                }
            }
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

            results.extend(download_client_status_health_results(
                &config.name,
                &status,
                has_remote_path_mappings,
                &library_roots,
            ));
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

    async fn check_recycle_bin_config(&self) -> Vec<HealthCheckResult> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let label = health_root_label(&facet);
            let root_folders = match self.root_folders_for_facet(&facet).await {
                Ok(root_folders) => root_folders,
                Err(error) => {
                    results.push(HealthCheckResult {
                        source: "RecycleBin".into(),
                        status: HealthCheckStatus::Error,
                        message: format!(
                            "Failed to resolve {label} roots while validating recycle bin config: {error}"
                        ),
                    });
                    continue;
                }
            };

            for root in root_folders {
                let path = root.path.trim();
                if path.is_empty() || !seen.insert(path.to_string()) {
                    continue;
                }

                let config = crate::recycle_bin::resolve_recycle_config(self, Some(path)).await;
                if config.enabled && !config.cleanup_enabled {
                    results.push(HealthCheckResult {
                        source: "RecycleBin".into(),
                        status: HealthCheckStatus::Error,
                        message: format!(
                            "Recycle bin cleanup is disabled for {label} root {path}: {}",
                            config
                                .validation_error
                                .as_deref()
                                .unwrap_or("invalid recycle bin configuration")
                        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_client_health_warns_for_inaccessible_mapped_roots() {
        let missing_root = std::env::temp_dir().join(format!(
            "scryer-health-missing-{}",
            scryer_domain::Id::new().0
        ));
        let status = DownloadClientStatus {
            is_localhost: Some(true),
            remote_output_roots: vec![missing_root.display().to_string()],
            ..DownloadClientStatus::default()
        };

        let results = download_client_status_health_results("Decypharr SAB", &status, true, &[]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "DownloadClient");
        assert_eq!(results[0].status, HealthCheckStatus::Warning);
        assert!(results[0].message.contains("Decypharr SAB"));
        assert!(
            results[0]
                .message
                .contains("still cannot access after remote path mapping")
        );
        assert!(
            results[0]
                .message
                .contains(missing_root.display().to_string().as_str())
        );
    }

    #[test]
    fn download_client_health_warns_for_overlapping_library_roots() {
        let status = DownloadClientStatus {
            is_localhost: Some(true),
            remote_output_roots: vec!["/srv/downloads/complete/series".to_string()],
            ..DownloadClientStatus::default()
        };

        let results = download_client_status_health_results(
            "Decypharr qBittorrent",
            &status,
            false,
            &["/srv/downloads/complete/series".to_string()],
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "DownloadClient");
        assert_eq!(results[0].status, HealthCheckStatus::Warning);
        assert!(results[0].message.contains("Decypharr qBittorrent"));
        assert!(results[0].message.contains("overlap library roots"));
        assert!(
            results[0]
                .message
                .contains("/srv/downloads/complete/series overlaps /srv/downloads/complete/series")
        );
    }
}
