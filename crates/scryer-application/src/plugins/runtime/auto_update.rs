/// Result of one scheduled automatic plugin-update cycle.
#[derive(Clone, Debug, Default)]
pub(crate) struct PluginAutoUpdateReport {
    pub(crate) enabled: bool,
    pub(crate) considered: usize,
    pub(crate) updated: Vec<PluginAutoUpdateUpgrade>,
    pub(crate) skipped_in_progress: Vec<String>,
    pub(crate) failed: Vec<PluginAutoUpdateFailure>,
    pub(crate) rollback_failures: Vec<PluginAutoUpdateRollbackFailure>,
    pub(crate) errors: Vec<String>,
}
impl PluginAutoUpdateReport {
    pub(crate) fn did_work(&self) -> bool {
        !self.updated.is_empty()
            || !self.skipped_in_progress.is_empty()
            || !self.failed.is_empty()
            || !self.errors.is_empty()
    }

    pub(crate) fn has_failures(&self) -> bool {
        !self.failed.is_empty() || !self.rollback_failures.is_empty() || !self.errors.is_empty()
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct PluginAutoUpdateUpgrade {
    pub(crate) plugin_id: String,
    pub(crate) from_version: String,
    pub(crate) to_version: String,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct PluginAutoUpdateFailure {
    pub(crate) plugin_id: String,
    pub(crate) error: String,
    pub(crate) rolled_back: bool,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct PluginAutoUpdateRollbackFailure {
    pub(crate) plugin_id: String,
    pub(crate) error: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginAutoUpdateKind {
    Patch,
    OptimizedArtifact,
}
#[derive(Clone, Debug, Default)]
struct PluginAutoUpdateRollback {
    restored: bool,
    error: Option<String>,
}
/// Eligibility of the release the catalog already selected for this plugin.
///
/// Automation never picks an alternative release: it either takes the same
/// candidate the update badge and a manual upgrade would take, or it skips.
fn plugin_auto_update_candidate_kind(
    installation: &PluginInstallation,
    resolved: &CatalogPluginResolution,
) -> Option<PluginAutoUpdateKind> {
    let selected = parse_catalog_release_version(&resolved.catalog_entry.id, &resolved.release)?;
    let installed = semver::Version::parse(installation.version.as_str()).ok()?;

    if !selected.pre.is_empty() {
        return None;
    }
    if selected.major == installed.major
        && selected.minor == installed.minor
        && selected.patch > installed.patch
    {
        return Some(PluginAutoUpdateKind::Patch);
    }
    if selected == installed
        && same_version_simd_artifact_update_available(
            installation,
            &resolved.release,
            &resolved.artifact,
        )
    {
        return Some(PluginAutoUpdateKind::OptimizedArtifact);
    }
    None
}
impl AppUseCase {
    async fn collect_plugin_auto_update_candidates(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, CatalogPluginResolution)>> {
        let mut installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?
            .into_iter()
            .filter(installation_is_catalog_official)
            .collect::<Vec<_>>();
        if installations.is_empty() {
            return Ok(Vec::new());
        }
        installations.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

        let resolved_by_id = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .map(|resolved| (resolved.catalog_entry.id.clone(), resolved))
            .collect::<HashMap<_, _>>();

        Ok(installations
            .into_iter()
            .filter_map(|installation| {
                let resolved = resolved_by_id.get(&installation.plugin_id)?;
                if !catalog_resolution_is_first_party(resolved) {
                    return None;
                }
                plugin_auto_update_candidate_kind(&installation, resolved)?;
                Some((installation, resolved.clone()))
            })
            .collect())
    }
}
impl AppUseCase {
    /// Applies eligible official-plugin updates after a scheduled catalog
    /// refresh. Infallible by construction: automation must never abort the
    /// catalog-refresh job, so every internal error becomes a report entry.
    pub(crate) async fn run_scheduled_plugin_auto_update(&self) -> PluginAutoUpdateReport {
        let mut report = PluginAutoUpdateReport::default();
        match self.load_plugin_auto_update_settings().await {
            Ok(settings) if settings.enabled => report.enabled = true,
            Ok(_) => return report,
            Err(error) => {
                warn!(
                    error = %error,
                    "skipping automatic plugin updates: settings are unavailable"
                );
                return report;
            }
        }

        let candidates = match self.collect_plugin_auto_update_candidates().await {
            Ok(candidates) => candidates,
            Err(error) => {
                warn!(error = %error, "automatic plugin update candidate selection failed");
                report.errors.push(error.to_string());
                return report;
            }
        };
        report.considered = candidates.len();

        let actor = User::system_execution_actor();
        for (installation, resolved) in candidates {
            let plugin_id = installation.plugin_id.clone();
            if self
                .runtime
                .plugins
                .plugin_install_orchestrator
                .begin(&actor.id, &plugin_id, PluginInstallOperationKind::Upgrade)
                .await
                .is_err()
            {
                // An interactive operation owns the slot; the next scheduled
                // cycle retries this plugin.
                report.skipped_in_progress.push(plugin_id);
                continue;
            }

            let reporter = PluginInstallProgressReporter::new(self, &actor.id, &plugin_id);
            let from_version = installation.version.clone();
            let mut rollback = PluginAutoUpdateRollback::default();
            match self
                .upgrade_catalog_plugin_with_rollback(
                    resolved,
                    installation,
                    &reporter,
                    &mut rollback,
                )
                .await
            {
                Ok(updated) => {
                    reporter.succeeded().await;
                    tracing::info!(
                        plugin_id = plugin_id.as_str(),
                        from_version = from_version.as_str(),
                        to_version = updated.version.as_str(),
                        "automatically updated official plugin"
                    );
                    report.updated.push(PluginAutoUpdateUpgrade {
                        plugin_id,
                        from_version,
                        to_version: updated.version,
                    });
                }
                Err(error) => {
                    reporter.failed(&error).await;
                    warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %error,
                        "automatic plugin update failed"
                    );
                    if let Some(rollback_error) = rollback.error {
                        report
                            .rollback_failures
                            .push(PluginAutoUpdateRollbackFailure {
                                plugin_id: plugin_id.clone(),
                                error: rollback_error,
                            });
                    }
                    report.failed.push(PluginAutoUpdateFailure {
                        plugin_id,
                        error: error.to_string(),
                        rolled_back: rollback.restored,
                    });
                }
            }
        }

        report
    }
}
impl AppUseCase {
    async fn upgrade_catalog_plugin_with_rollback(
        &self,
        resolved: CatalogPluginResolution,
        installation: PluginInstallation,
        reporter: &PluginInstallProgressReporter,
        rollback: &mut PluginAutoUpdateRollback,
    ) -> AppResult<PluginInstallation> {
        let prior_record = installation.clone();
        // Automation only touches a plugin it can put back: without the prior
        // artifact in hand there is nothing to compensate a failed upgrade with.
        let prior_wasm = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation_wasm_payload(&prior_record.plugin_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "plugin '{}' is missing persisted WASM payload",
                    prior_record.plugin_id
                ))
            })?;

        let error = match self
            .upgrade_catalog_plugin(resolved, installation, reporter)
            .await
        {
            Ok(updated) => return Ok(updated),
            Err(error) => error,
        };

        match self
            .restore_plugin_installation_snapshot(&prior_record, &prior_wasm)
            .await
        {
            Ok(()) => rollback.restored = true,
            Err(rollback_error) => {
                warn!(
                    plugin_id = prior_record.plugin_id.as_str(),
                    error = %rollback_error,
                    "failed to roll back automatic plugin update"
                );
                rollback.error = Some(rollback_error.to_string());
            }
        }

        Err(error)
    }
}
impl AppUseCase {
    async fn restore_plugin_installation_snapshot(
        &self,
        prior_record: &PluginInstallation,
        prior_wasm: &PersistedPluginWasmPayload,
    ) -> AppResult<()> {
        // A failed upgrade can leave the runtime holding a replacement under a
        // different plugin/provider type; drop that before the prior one is
        // re-registered.
        let replacement_runtime_types = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(&prior_record.plugin_id)
            .await
            .ok()
            .flatten()
            .filter(|current| {
                current.plugin_type != prior_record.plugin_type
                    || current.provider_type != prior_record.provider_type
            })
            .map(|current| (current.plugin_type, current.provider_type));
        if let Some((plugin_type, provider_type)) = replacement_runtime_types.as_ref()
            && let Err(error) = self.apply_runtime_plugin_removal_for_values(plugin_type, provider_type)
        {
            warn!(
                plugin_id = prior_record.plugin_id.as_str(),
                error = %error,
                "failed to remove replacement plugin runtime registration during rollback"
            );
        }

        let restored = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(prior_record, Some(prior_wasm.bytes.as_slice()))
            .await?;

        if restored.is_enabled {
            let runtime_plugin = self.load_runtime_plugin_for_installation(&restored).await?;
            self.apply_runtime_plugin_upsert(&restored, runtime_plugin)?;
        }

        let mut plugin_types = vec![restored.plugin_type.clone()];
        if let Some((plugin_type, _)) = replacement_runtime_types {
            plugin_types.push(plugin_type);
        }
        self.finalize_runtime_plugin_mutation_for_types(
            plugin_types.iter().map(String::as_str),
            restored.is_enabled || plugin_types.len() > 1,
        )
        .await
    }
}
