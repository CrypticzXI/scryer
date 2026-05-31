#[derive(Clone)]
struct PluginInstallProgressReporter {
    orchestrator: crate::services::PluginInstallOrchestrator,
    actor_user_id: String,
    plugin_id: String,
}
impl PluginInstallProgressReporter {
    fn new(app: &AppUseCase, actor_user_id: &str, plugin_id: &str) -> Self {
        Self {
            orchestrator: app.runtime.plugins.plugin_install_orchestrator.clone(),
            actor_user_id: actor_user_id.to_string(),
            plugin_id: plugin_id.to_string(),
        }
    }

    async fn downloading(&self) {
        self.transition(PluginInstallState::Downloading, None, None)
            .await;
    }

    async fn verifying(&self) {
        self.transition(PluginInstallState::Verifying, None, None)
            .await;
    }

    async fn installing(&self) {
        self.transition(PluginInstallState::Installing, None, None)
            .await;
    }

    async fn succeeded(&self) {
        self.transition(PluginInstallState::Succeeded, None, None)
            .await;
    }

    async fn failed(&self, error: &AppError) {
        self.transition(PluginInstallState::Failed, None, Some(error.to_string()))
            .await;
    }

    async fn transition(
        &self,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) {
        self.orchestrator
            .transition(&self.actor_user_id, &self.plugin_id, state, message, error)
            .await;
    }
}
impl AppUseCase {
    async fn fetch_catalog_release_wasm(
        &self,
        resolved: &CatalogPluginResolution,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<FetchedCatalogArtifact> {
        reporter.downloading().await;
        let signer = if resolved.source_kind == PluginSourceKind::Downloaded {
            resolved.catalog_entry.required_signer.clone()
        } else {
            RequiredSigner {
                github_repository: resolved.github_repo.slug(),
                github_workflow: None,
            }
        };
        let data_urls = primary_and_mirrors(&resolved.artifact.url, &resolved.artifact.mirror_urls);
        let signature_urls = primary_and_mirrors(
            &resolved.artifact.signature_url,
            &resolved.artifact.signature_mirror_urls,
        );
        let (compressed_artifact, artifact_url) = self
            .fetch_verified_blob_from_locations(
                &data_urls,
                &signature_urls,
                &signer,
                "plugin artifact",
            )
            .await?;
        reporter.verifying().await;
        verify_digest_set(
            "compressed plugin artifact",
            &resolved.artifact.digests,
            &compressed_artifact,
        )?;
        let wasm_encoding = match artifact_encoding_from_url(&artifact_url) {
            Some("br") => PluginWasmEncoding::Brotli,
            Some("zst") => PluginWasmEncoding::Zstd,
            _ => {
                return Err(AppError::Validation(format!(
                    "plugin '{}' selected artifact '{}' has unsupported encoding",
                    resolved.catalog_entry.id, artifact_url
                )));
            }
        };
        let wasm = match wasm_encoding {
            PluginWasmEncoding::Brotli => decompress_brotli(compressed_artifact.clone()).await?,
            PluginWasmEncoding::Zstd => decompress_zstd(compressed_artifact.clone()).await?,
            PluginWasmEncoding::Identity => compressed_artifact.clone(),
        };
        verify_digest_set(
            "decompressed plugin WASM",
            &resolved.artifact.wasm_digests,
            &wasm,
        )?;
        let actual_bytes = u64::try_from(wasm.len()).map_err(|_| {
            AppError::Validation(format!(
                "plugin '{}' selected artifact '{}' is too large to validate bytes",
                resolved.catalog_entry.id, artifact_url
            ))
        })?;
        if actual_bytes != resolved.artifact.bytes {
            return Err(AppError::Validation(format!(
                "plugin '{}' selected artifact '{}' bytes mismatch: expected {}, got {}",
                resolved.catalog_entry.id, artifact_url, resolved.artifact.bytes, actual_bytes
            )));
        }
        Ok(FetchedCatalogArtifact {
            persisted_wasm_bytes: compressed_artifact,
            wasm_bytes: wasm,
            artifact_url,
            artifact_digest: blake3_digest_string(
                &resolved.artifact.digests,
                "compressed plugin artifact",
            )?,
            wasm_encoding,
        })
    }
}
impl AppUseCase {
    async fn prepare_catalog_plugin_install(
        &self,
        resolved: &CatalogPluginResolution,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PreparedCatalogPluginInstall> {
        if is_reserved_first_party_provider(&resolved.catalog_entry.provider_type) {
            return Err(AppError::Validation(format!(
                "provider type '{}' is reserved for first-party code",
                resolved.catalog_entry.provider_type
            )));
        }

        let fetched = self.fetch_catalog_release_wasm(resolved, reporter).await?;
        let release = DownloadedPluginReleaseContract {
            version: resolved.release.version.clone(),
            sdk_version: None,
            sdk_constraint: resolved.release.sdk_constraint.clone(),
            scryer_constraint: catalog_release_scryer_constraint(&resolved.release),
        };
        let validated = self.validate_catalog_downloaded_plugin_release(
            &resolved.catalog_entry.id,
            &resolved.catalog_entry.plugin_type,
            &resolved.catalog_entry.provider_type,
            &release,
            &fetched.wasm_bytes,
        )?;
        let (wasm_digest_algo, wasm_digest) =
            blake3_digest_components(&resolved.artifact.wasm_digests, "plugin artifact WASM")?;

        Ok(PreparedCatalogPluginInstall {
            descriptor: validated.descriptor,
            sdk_constraint: validated.sdk_constraint,
            scryer_constraint: release.scryer_constraint,
            source_kind: resolved.source_kind,
            support_tier: resolved.effective_support_tier,
            persisted_wasm_bytes: fetched.persisted_wasm_bytes,
            runtime_wasm_bytes: fetched.wasm_bytes,
            runtime_first_party: catalog_resolution_is_first_party(resolved),
            wasm_encoding: fetched.wasm_encoding,
            wasm_digest_algo,
            source_url: fetched.artifact_url.clone(),
            publisher: resolved.catalog_entry.publisher.clone(),
            docs_url: resolved.catalog_entry.docs_url.clone(),
            source_repo: resolved.catalog_entry.source_repo.clone(),
            manifest_url: fetched.artifact_url,
            wasm_digest,
            artifact_digest: fetched.artifact_digest,
            description: resolved.catalog_entry.description.clone(),
        })
    }
}
impl AppUseCase {
    async fn install_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        let prepared = self
            .prepare_catalog_plugin_install(&resolved, reporter)
            .await?;
        let persisted_wasm_bytes = prepared.persisted_wasm_bytes.clone();
        let (installation, runtime_plugin) =
            prepared.into_new_installation(resolved.catalog_entry.id.clone())?;

        reporter.installing().await;
        let result = self
            .services
            .customization
            .plugin_installations
            .create_plugin_installation(&installation, Some(persisted_wasm_bytes.as_slice()))
            .await?;

        self.apply_runtime_plugin_upsert(&result, runtime_plugin)?;
        self.finalize_runtime_plugin_mutation(&result.plugin_type, true)
            .await?;
        Ok(result)
    }
}
impl AppUseCase {
    async fn upgrade_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
        installation: PluginInstallation,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        let selected_version = semver::Version::parse(
            resolved.release.version.trim_start_matches('v'),
        )
        .map_err(|e| {
            AppError::Validation(format!(
                "invalid catalog version '{}': {e}",
                resolved.release.version
            ))
        })?;
        let installed_version = semver::Version::parse(&installation.version).map_err(|e| {
            AppError::Validation(format!(
                "invalid installed version '{}': {e}",
                installation.version
            ))
        })?;
        if selected_version <= installed_version {
            return Err(AppError::Validation(format!(
                "plugin '{}' is already at version {} (selected release is {})",
                resolved.catalog_entry.id, installation.version, resolved.release.version
            )));
        }

        let prepared = self
            .prepare_catalog_plugin_install(&resolved, reporter)
            .await?;
        let persisted_wasm_bytes = prepared.persisted_wasm_bytes.clone();
        let previous_plugin_type = installation.plugin_type.clone();
        let previous_provider_type = installation.provider_type.clone();
        let (updated, runtime_plugin) = prepared.into_updated_installation(installation)?;

        reporter.installing().await;
        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(persisted_wasm_bytes.as_slice()))
            .await?;

        let runtime_touched = result.is_enabled;
        if runtime_touched {
            let mut previous_runtime_installation = result.clone();
            previous_runtime_installation.plugin_type = previous_plugin_type.clone();
            previous_runtime_installation.provider_type = previous_provider_type.clone();
            self.apply_runtime_plugin_replace(
                &previous_runtime_installation,
                &result,
                runtime_plugin,
            )?;
        }
        self.finalize_runtime_plugin_mutation_for_types(
            [previous_plugin_type.as_str(), result.plugin_type.as_str()],
            runtime_touched,
        )
        .await?;
        Ok(result)
    }
}
impl AppUseCase {
    pub async fn migrate_nzbgeek_builtin_to_official_internal(&self) -> AppResult<()> {
        let Some(installation) = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(LEGACY_NZBGEEK_PLUGIN_ID)
            .await?
        else {
            return Ok(());
        };

        if !preserves_legacy_nzbgeek_builtin_for_catalog_migration(&installation) {
            return Ok(());
        }

        let resolved = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|resolved| {
                resolved.catalog_entry.id == LEGACY_NZBGEEK_PLUGIN_ID
                    && resolved.source_kind == PluginSourceKind::Downloaded
                    && resolved.effective_support_tier == PluginSupportTier::Official
            })
            .ok_or_else(|| {
                AppError::NotFound(
                    "official nzbgeek plugin is not available for builtin migration".to_string(),
                )
            })?;
        let reporter = PluginInstallProgressReporter::new(self, "system", LEGACY_NZBGEEK_PLUGIN_ID);
        let prepared = self
            .prepare_catalog_plugin_install(&resolved, &reporter)
            .await?;
        let persisted_wasm_bytes = prepared.persisted_wasm_bytes.clone();
        let previous_plugin_type = installation.plugin_type.clone();
        let previous_provider_type = installation.provider_type.clone();
        let (updated, runtime_plugin) = prepared.into_updated_installation(installation)?;

        reporter.installing().await;
        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(persisted_wasm_bytes.as_slice()))
            .await?;

        let runtime_touched = result.is_enabled;
        if runtime_touched {
            let mut previous_runtime_installation = result.clone();
            previous_runtime_installation.plugin_type = previous_plugin_type.clone();
            previous_runtime_installation.provider_type = previous_provider_type.clone();
            self.apply_runtime_plugin_replace(
                &previous_runtime_installation,
                &result,
                runtime_plugin,
            )?;
        }
        self.finalize_runtime_plugin_mutation_for_types(
            [previous_plugin_type.as_str(), result.plugin_type.as_str()],
            runtime_touched,
        )
        .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn install_manual_plugin(
        &self,
        actor: &User,
        github_repo_url: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (resolved, catalog_json) = self.resolve_manual_plugin_repo(github_repo_url).await?;
        if is_reserved_first_party_provider(&resolved.catalog_entry.provider_type) {
            return Err(AppError::Validation(format!(
                "provider type '{}' is reserved for first-party code",
                resolved.catalog_entry.provider_type
            )));
        }
        let _operation_guard = self
            .runtime
            .plugins
            .plugin_operation_guards
            .acquire(&resolved.catalog_entry.id)
            .await;
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(&resolved.catalog_entry.id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{}' is already installed",
                resolved.catalog_entry.id
            )));
        }
        let catalog_url = resolved.github_repo.catalog_v3_url();
        self.upsert_manual_plugin_catalog_source(
            &resolved.github_repo,
            &catalog_url,
            Some(catalog_json),
            None,
        )
        .await?;
        let reporter =
            PluginInstallProgressReporter::new(self, &actor.id, &resolved.catalog_entry.id);
        self.install_catalog_plugin(resolved, &reporter).await
    }
}
impl AppUseCase {
    async fn prepare_restored_plugin_recovery(
        &self,
        target: RestoredPluginRecoveryTarget,
        resolved: CatalogPluginResolution,
    ) -> AppResult<PreparedRestoredPluginRecovery> {
        let reporter = PluginInstallProgressReporter::new(
            self,
            RESTORE_PLUGIN_RECOVERY_ACTOR_ID,
            &target.installation.plugin_id,
        );
        let prepared = self
            .prepare_catalog_plugin_install(&resolved, &reporter)
            .await?;
        let persisted_wasm_bytes = prepared.persisted_wasm_bytes.clone();
        let (updated_installation, _) = prepared.into_updated_installation(target.installation)?;
        Ok(PreparedRestoredPluginRecovery {
            updated_installation,
            persisted_wasm_bytes,
        })
    }
}
impl AppUseCase {
    async fn perform_catalog_install(
        &self,
        plugin_id: &str,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already installed"
            )));
        }

        if let Some(resolved) = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.catalog_entry.id == plugin_id)
        {
            return self.install_catalog_plugin(resolved, reporter).await;
        }
        Err(AppError::NotFound(format!(
            "plugin '{plugin_id}' is not available from the plugin catalog"
        )))
    }
}
impl AppUseCase {
    async fn perform_catalog_upgrade(
        &self,
        plugin_id: &str,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.source_kind == PluginSourceKind::Manual {
            let source_repo = installation.source_repo.as_deref().ok_or_else(|| {
                AppError::Validation(format!(
                    "manual plugin '{plugin_id}' is missing source repo"
                ))
            })?;
            let (resolved, catalog_json) = self.resolve_manual_plugin_repo(source_repo).await?;
            let catalog_url = resolved.github_repo.catalog_v3_url();
            self.upsert_manual_plugin_catalog_source(
                &resolved.github_repo,
                &catalog_url,
                Some(catalog_json),
                None,
            )
            .await?;
            return self
                .upgrade_catalog_plugin(resolved, installation, reporter)
                .await;
        }

        if let Some(resolved) = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.catalog_entry.id == plugin_id)
        {
            return self
                .upgrade_catalog_plugin(resolved, installation, reporter)
                .await;
        }
        Err(AppError::NotFound(format!(
            "plugin '{plugin_id}' is not available from the plugin catalog"
        )))
    }
}
impl AppUseCase {
    pub async fn begin_install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_install_request(plugin_id).await?;
        let snapshot = self
            .runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Install)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let app = self.clone();
        let actor = actor.clone();
        let plugin_id = plugin_id.trim().to_string();
        tokio::spawn(async move {
            let reporter = PluginInstallProgressReporter::new(&app, &actor.id, &plugin_id);
            let result = app.perform_catalog_install(&plugin_id, &reporter).await;
            match result {
                Ok(_) => reporter.succeeded().await,
                Err(error) => {
                    reporter.failed(&error).await;
                    tracing::warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %error,
                        "plugin install operation failed"
                    );
                }
            }
        });
        Ok(snapshot)
    }
}
impl AppUseCase {
    pub async fn begin_upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_upgrade_request(plugin_id).await?;
        let snapshot = self
            .runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Upgrade)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let app = self.clone();
        let actor = actor.clone();
        let plugin_id = plugin_id.trim().to_string();
        tokio::spawn(async move {
            let reporter = PluginInstallProgressReporter::new(&app, &actor.id, &plugin_id);
            let result = app.perform_catalog_upgrade(&plugin_id, &reporter).await;
            match result {
                Ok(_) => reporter.succeeded().await,
                Err(error) => {
                    reporter.failed(&error).await;
                    tracing::warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %error,
                        "plugin upgrade operation failed"
                    );
                }
            }
        });
        Ok(snapshot)
    }
}
impl AppUseCase {
    /// Install a plugin from the registry.
    pub async fn install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_install_request(plugin_id).await?;
        self.runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Install)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let reporter = PluginInstallProgressReporter::new(self, &actor.id, plugin_id);
        let result = self.perform_catalog_install(plugin_id, &reporter).await;
        match &result {
            Ok(_) => reporter.succeeded().await,
            Err(error) => reporter.failed(error).await,
        }
        result
    }
}
impl AppUseCase {
    /// Upgrade a non-builtin plugin to the latest registry version.
    pub async fn upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_upgrade_request(plugin_id).await?;
        self.runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Upgrade)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let reporter = PluginInstallProgressReporter::new(self, &actor.id, plugin_id);
        let result = self.perform_catalog_upgrade(plugin_id, &reporter).await;
        match &result {
            Ok(_) => reporter.succeeded().await,
            Err(error) => reporter.failed(error).await,
        }
        result
    }
}
