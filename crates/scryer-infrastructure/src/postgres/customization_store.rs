use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, persisted_records::external_plugin_installation_is_supported_shape,
};
use scryer_domain::{
    Id, MediaFacet, PersistedPluginWasmPayload, PluginCatalogSource, PluginCatalogStatusRecord,
    PluginInstallation, PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
    PostProcessingScript, PostProcessingScriptRun, RuleSet,
};
use serde_json::Value;
use sqlx::Row;

use crate::customization_store::{CustomizationSql, CustomizationStore};
use crate::postgres::timestamp::{parse_optional_rfc3339_timestamp, parse_rfc3339_timestamp};

pub type PostgresCustomizationStore = CustomizationStore<PostgresCustomizationSql>;

#[derive(Clone)]
pub struct PostgresCustomizationSql {
    pool: sqlx::PgPool,
}

impl CustomizationStore<PostgresCustomizationSql> {
    pub fn new(db: &super::PostgresServices) -> Self {
        Self::from_sql(PostgresCustomizationSql::new(db.pool().clone()))
    }
}

impl PostgresCustomizationSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    async fn prune_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>> {
        let rows = sqlx::query(&format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              WHERE is_builtin = FALSE AND source_kind IN ('downloaded', 'manual')"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        let mut removed_plugin_ids = Vec::new();
        for row in rows {
            if row_is_incompatible_external_installation(&row) {
                removed_plugin_ids.push(row.try_get("plugin_id").map_err(repo_err)?);
            }
        }

        if !removed_plugin_ids.is_empty() {
            sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ANY($1)")
                .bind(&removed_plugin_ids)
                .execute(&self.pool)
                .await
                .map_err(repo_err)?;
        }

        Ok(removed_plugin_ids)
    }

    async fn upsert_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let descriptor_json = optional_json_value(installation.descriptor_json.as_deref())?;
        sqlx::query(
            "INSERT INTO plugin_installations (
                id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, source_kind, is_enabled,
                is_builtin, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url,
                support_tier, publisher, docs_url, source_repo, manifest_url, wasm_digest,
                artifact_digest, descriptor_json, installed_at, updated_at
             )
             VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17,
                $18, $19, $20, $21, $22, $23,
                $24, $25::jsonb, $26, $27
             )
             ON CONFLICT (plugin_id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                version = EXCLUDED.version,
                sdk_version = EXCLUDED.sdk_version,
                sdk_constraint = EXCLUDED.sdk_constraint,
                scryer_constraint = EXCLUDED.scryer_constraint,
                plugin_type = EXCLUDED.plugin_type,
                provider_type = EXCLUDED.provider_type,
                source_kind = EXCLUDED.source_kind,
                is_enabled = EXCLUDED.is_enabled,
                is_builtin = EXCLUDED.is_builtin,
                wasm_bytes = COALESCE(EXCLUDED.wasm_bytes, plugin_installations.wasm_bytes),
                wasm_encoding = EXCLUDED.wasm_encoding,
                wasm_digest_algo = EXCLUDED.wasm_digest_algo,
                source_url = EXCLUDED.source_url,
                support_tier = EXCLUDED.support_tier,
                publisher = EXCLUDED.publisher,
                docs_url = EXCLUDED.docs_url,
                source_repo = EXCLUDED.source_repo,
                manifest_url = EXCLUDED.manifest_url,
                wasm_digest = EXCLUDED.wasm_digest,
                artifact_digest = EXCLUDED.artifact_digest,
                descriptor_json = EXCLUDED.descriptor_json,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&installation.id)
        .bind(&installation.plugin_id)
        .bind(&installation.name)
        .bind(&installation.description)
        .bind(&installation.version)
        .bind(&installation.sdk_version)
        .bind(&installation.sdk_constraint)
        .bind(&installation.scryer_constraint)
        .bind(&installation.plugin_type)
        .bind(&installation.provider_type)
        .bind(source_kind_label(installation.source_kind))
        .bind(installation.is_enabled)
        .bind(installation.is_builtin)
        .bind(wasm_bytes.map(|bytes| bytes.to_vec()))
        .bind(wasm_encoding_label(installation.wasm_encoding))
        .bind(&installation.wasm_digest_algo)
        .bind(&installation.source_url)
        .bind(support_tier_label(installation.support_tier))
        .bind(&installation.publisher)
        .bind(&installation.docs_url)
        .bind(&installation.source_repo)
        .bind(&installation.manifest_url)
        .bind(&installation.wasm_digest)
        .bind(&installation.artifact_digest)
        .bind(descriptor_json)
        .bind(installation.installed_at)
        .bind(installation.updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(installation.clone())
    }

    async fn upsert_post_processing_script(&self, script: &PostProcessingScript) -> AppResult<()> {
        let applied_facets = serde_json::to_value(&script.applied_facets).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO post_processing_scripts
             (id, name, description, script_type, script_content, applied_facets,
              execution_mode, timeout_secs, priority, enabled, debug, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                script_type = EXCLUDED.script_type,
                script_content = EXCLUDED.script_content,
                applied_facets = EXCLUDED.applied_facets,
                execution_mode = EXCLUDED.execution_mode,
                timeout_secs = EXCLUDED.timeout_secs,
                priority = EXCLUDED.priority,
                enabled = EXCLUDED.enabled,
                debug = EXCLUDED.debug,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&script.id)
        .bind(&script.name)
        .bind(&script.description)
        .bind(script.script_type.as_str())
        .bind(&script.script_content)
        .bind(applied_facets)
        .bind(script.execution_mode.as_str())
        .bind(script.timeout_secs)
        .bind(script.priority)
        .bind(script.enabled)
        .bind(script.debug)
        .bind(script.created_at)
        .bind(script.updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl CustomizationSql for PostgresCustomizationSql {
    async fn delete_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>> {
        self.prune_incompatible_external_plugin_installations()
            .await
    }

    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let rows = sqlx::query(&format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets ORDER BY priority DESC, name ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_rule_set).collect()
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let rows = sqlx::query(&format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE enabled = TRUE ORDER BY priority DESC, name ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_rule_set).collect()
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let row = sqlx::query(&format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_rule_set).transpose()
    }

    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        self.update_rule_set(rule_set).await
    }

    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let applied_facets = serde_json::to_value(&rule_set.applied_facets).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO rule_sets
             (id, name, description, rego_source, enabled, priority, applied_facets,
              created_at, updated_at, is_managed, managed_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8, $9, $10, $11)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                rego_source = EXCLUDED.rego_source,
                enabled = EXCLUDED.enabled,
                priority = EXCLUDED.priority,
                applied_facets = EXCLUDED.applied_facets,
                updated_at = EXCLUDED.updated_at,
                is_managed = EXCLUDED.is_managed,
                managed_key = EXCLUDED.managed_key",
        )
        .bind(&rule_set.id)
        .bind(&rule_set.name)
        .bind(&rule_set.description)
        .bind(&rule_set.rego_source)
        .bind(rule_set.enabled)
        .bind(rule_set.priority)
        .bind(applied_facets)
        .bind(rule_set.created_at)
        .bind(rule_set.updated_at)
        .bind(rule_set.is_managed)
        .bind(&rule_set.managed_key)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM rule_sets WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        let event_json = serde_json::json!({
            "action": action,
            "rego_source": rego_source,
            "actor_id": actor_id,
        });
        sqlx::query(
            "INSERT INTO rule_set_history (id, rule_set_id, event_json, created_at)
             VALUES ($1, $2, $3::jsonb, NOW())",
        )
        .bind(Id::new().0)
        .bind(rule_set_id)
        .bind(event_json)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        let row = sqlx::query(&format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE managed_key = $1 LIMIT 1"
        ))
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_rule_set).transpose()
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM rule_sets WHERE managed_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        let pattern = format!("{prefix}%");
        let rows = sqlx::query(&format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE managed_key LIKE $1 ORDER BY managed_key"
        ))
        .bind(pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_rule_set).collect()
    }

    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        let rows = sqlx::query(&format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS} FROM post_processing_scripts ORDER BY priority ASC, name"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_post_processing_script).collect()
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        let row = sqlx::query(&format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS} FROM post_processing_scripts WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_post_processing_script).transpose()
    }

    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.upsert_post_processing_script(&script).await?;
        Ok(script)
    }

    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.upsert_post_processing_script(&script).await?;
        Ok(script)
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM post_processing_scripts WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>> {
        let rows = sqlx::query(&format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              WHERE enabled = TRUE
                AND (applied_facets = '[]'::jsonb OR applied_facets ? $1)
              ORDER BY priority ASC, name"
        ))
        .bind(facet)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_post_processing_script).collect()
    }

    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        let started_at =
            parse_rfc3339_timestamp(&run.started_at, "post_processing_script_runs.started_at")?;
        let completed_at = parse_optional_rfc3339_timestamp(
            run.completed_at.as_deref(),
            "post_processing_script_runs.completed_at",
        )?;
        sqlx::query(
            "INSERT INTO post_processing_script_runs
             (id, script_id, script_name, title_id, title_name, facet, file_path,
              status, exit_code, stdout_tail, stderr_tail, duration_ms, env_payload_json,
              started_at, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (id) DO UPDATE SET
                script_name = EXCLUDED.script_name,
                title_id = EXCLUDED.title_id,
                title_name = EXCLUDED.title_name,
                facet = EXCLUDED.facet,
                file_path = EXCLUDED.file_path,
                status = EXCLUDED.status,
                exit_code = EXCLUDED.exit_code,
                stdout_tail = EXCLUDED.stdout_tail,
                stderr_tail = EXCLUDED.stderr_tail,
                duration_ms = EXCLUDED.duration_ms,
                env_payload_json = EXCLUDED.env_payload_json,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at",
        )
        .bind(&run.id)
        .bind(&run.script_id)
        .bind(&run.script_name)
        .bind(&run.title_id)
        .bind(&run.title_name)
        .bind(&run.facet)
        .bind(&run.file_path)
        .bind(run.status.as_str())
        .bind(run.exit_code)
        .bind(&run.stdout_tail)
        .bind(&run.stderr_tail)
        .bind(run.duration_ms)
        .bind(&run.env_payload_json)
        .bind(started_at)
        .bind(completed_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let rows = sqlx::query(&format!(
            "SELECT {POST_PROCESSING_RUN_COLUMNS}
               FROM post_processing_script_runs
              WHERE script_id = $1
              ORDER BY started_at DESC
              LIMIT $2"
        ))
        .bind(script_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_post_processing_run).collect()
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let rows = sqlx::query(&format!(
            "SELECT {POST_PROCESSING_RUN_COLUMNS}
               FROM post_processing_script_runs
              WHERE title_id = $1
              ORDER BY started_at DESC
              LIMIT $2"
        ))
        .bind(title_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_post_processing_run).collect()
    }

    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        let rows = sqlx::query(&format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              ORDER BY is_builtin DESC, name, plugin_id"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .filter(|row| !row_is_incompatible_external_installation(row))
            .map(row_to_plugin_installation)
            .collect()
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let row = sqlx::query(&format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              WHERE plugin_id = $1"
        ))
        .bind(plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref()
            .filter(|row| !row_is_incompatible_external_installation(row))
            .map(row_to_plugin_installation)
            .transpose()
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.upsert_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.upsert_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = $1")
            .bind(plugin_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        let rows = sqlx::query(&format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              WHERE is_enabled = TRUE
              ORDER BY is_builtin DESC, name, plugin_id"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        rows.iter()
            .map(|row| {
                let installation = row_to_plugin_installation(row)?;
                let bytes: Option<Vec<u8>> = row.try_get("wasm_bytes").map_err(repo_err)?;
                let payload = bytes.map(|bytes| PersistedPluginWasmPayload {
                    encoding: installation.wasm_encoding,
                    bytes,
                });
                Ok((installation, payload))
            })
            .collect()
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        let row = sqlx::query(
            "SELECT wasm_bytes, wasm_encoding FROM plugin_installations WHERE plugin_id = $1",
        )
        .bind(plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        let Some(row) = row else {
            return Ok(None);
        };
        let bytes: Option<Vec<u8>> = row.try_get("wasm_bytes").map_err(repo_err)?;
        let encoding_raw: String = row.try_get("wasm_encoding").map_err(repo_err)?;
        Ok(bytes.map(|bytes| PersistedPluginWasmPayload {
            encoding: parse_wasm_encoding(&encoding_raw),
            bytes,
        }))
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        let existing = self.get_plugin_installation(plugin_id).await?;
        if existing
            .as_ref()
            .is_some_and(|installation| !installation.is_builtin)
        {
            return Ok(());
        }

        let now = Utc::now();
        let installation = PluginInstallation {
            id: existing
                .as_ref()
                .map(|installation| installation.id.clone())
                .unwrap_or_else(|| Id::new().0),
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            version: version.to_string(),
            sdk_version: sdk_version.to_string(),
            sdk_constraint: sdk_constraint.to_string(),
            scryer_constraint: None,
            plugin_type: plugin_type.to_string(),
            provider_type: provider_type.to_string(),
            source_kind: PluginSourceKind::Bundled,
            is_enabled: true,
            is_builtin: true,
            wasm_encoding: PluginWasmEncoding::Identity,
            wasm_digest_algo: None,
            source_url: None,
            support_tier: PluginSupportTier::Official,
            publisher: None,
            docs_url: None,
            source_repo: None,
            manifest_url: None,
            wasm_digest: None,
            artifact_digest: None,
            descriptor_json: None,
            installed_at: existing
                .as_ref()
                .map(|installation| installation.installed_at)
                .unwrap_or(now),
            updated_at: now,
        };
        self.upsert_plugin_installation(&installation, None).await?;
        Ok(())
    }

    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO plugin_catalog_sources (
                source_key, source_kind, source_url, github_repo, support_tier,
                catalog_json, last_success_at, last_error, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (source_key) DO UPDATE SET
                source_kind = EXCLUDED.source_kind,
                source_url = EXCLUDED.source_url,
                github_repo = EXCLUDED.github_repo,
                support_tier = EXCLUDED.support_tier,
                catalog_json = EXCLUDED.catalog_json,
                last_success_at = EXCLUDED.last_success_at,
                last_error = EXCLUDED.last_error,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&source.source_key)
        .bind(&source.source_kind)
        .bind(&source.source_url)
        .bind(&source.github_repo)
        .bind(support_tier_label(source.support_tier))
        .bind(&source.catalog_json)
        .bind(source.last_success_at)
        .bind(&source.last_error)
        .bind(source.updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>> {
        let rows = sqlx::query(&format!(
            "SELECT {PLUGIN_CATALOG_SOURCE_COLUMNS}
               FROM plugin_catalog_sources
              ORDER BY source_kind ASC, source_key ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_plugin_catalog_source).collect()
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>> {
        let row = sqlx::query(&format!(
            "SELECT {PLUGIN_CATALOG_SOURCE_COLUMNS}
               FROM plugin_catalog_sources
              WHERE source_key = $1"
        ))
        .bind(source_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_plugin_catalog_source).transpose()
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO plugin_catalog_status (status_key, status_json, checked_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (status_key) DO UPDATE SET
                status_json = EXCLUDED.status_json,
                checked_at = EXCLUDED.checked_at",
        )
        .bind(&status.status_key)
        .bind(&status.status_json)
        .bind(status.checked_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>> {
        let row = sqlx::query(
            "SELECT status_key, status_json, checked_at
               FROM plugin_catalog_status
              WHERE status_key = $1",
        )
        .bind(status_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_plugin_catalog_status).transpose()
    }
}

const RULE_SET_COLUMNS: &str = "id, name, description, rego_source, enabled, priority,
    applied_facets, created_at, updated_at, is_managed, managed_key";

const POST_PROCESSING_SCRIPT_COLUMNS: &str = "id, name, description, script_type, script_content,
    applied_facets, execution_mode, timeout_secs, priority, enabled, debug, created_at, updated_at";

const POST_PROCESSING_RUN_COLUMNS: &str = "id, script_id, script_name, title_id, title_name, facet,
    file_path, status, exit_code, stdout_tail, stderr_tail, duration_ms, env_payload_json,
    started_at, completed_at";

const PLUGIN_INSTALLATION_COLUMNS: &str = "id, plugin_id, name, description, version, sdk_version,
    sdk_constraint, scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
    source_kind, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
    docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
    installed_at, updated_at";

const PLUGIN_CATALOG_SOURCE_COLUMNS: &str = "source_key, source_kind, source_url, github_repo,
    support_tier, catalog_json, last_success_at, last_error, updated_at";

fn row_to_rule_set(row: &sqlx::postgres::PgRow) -> AppResult<RuleSet> {
    let facets_value: Value = row.try_get("applied_facets").map_err(repo_err)?;
    let applied_facets = serde_json::from_value::<Vec<MediaFacet>>(facets_value)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(RuleSet {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        description: row.try_get("description").map_err(repo_err)?,
        rego_source: row.try_get("rego_source").map_err(repo_err)?,
        enabled: row.try_get("enabled").map_err(repo_err)?,
        priority: row.try_get("priority").map_err(repo_err)?,
        applied_facets,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
        is_managed: row.try_get("is_managed").map_err(repo_err)?,
        managed_key: row.try_get("managed_key").map_err(repo_err)?,
    })
}

fn row_to_post_processing_script(row: &sqlx::postgres::PgRow) -> AppResult<PostProcessingScript> {
    let facets_value: Value = row.try_get("applied_facets").map_err(repo_err)?;
    let applied_facets = serde_json::from_value(facets_value).unwrap_or_default();
    let script_type_raw: String = row.try_get("script_type").map_err(repo_err)?;
    let execution_mode_raw: String = row.try_get("execution_mode").map_err(repo_err)?;
    Ok(PostProcessingScript {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        description: row.try_get("description").map_err(repo_err)?,
        script_type: scryer_domain::ScriptType::parse(&script_type_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid script_type: {script_type_raw}"))
        })?,
        script_content: row.try_get("script_content").map_err(repo_err)?,
        applied_facets,
        execution_mode: scryer_domain::ExecutionMode::parse(&execution_mode_raw).ok_or_else(
            || AppError::Repository(format!("invalid execution_mode: {execution_mode_raw}")),
        )?,
        timeout_secs: row.try_get("timeout_secs").map_err(repo_err)?,
        priority: row.try_get("priority").map_err(repo_err)?,
        enabled: row.try_get("enabled").map_err(repo_err)?,
        debug: row.try_get("debug").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn row_to_post_processing_run(row: &sqlx::postgres::PgRow) -> AppResult<PostProcessingScriptRun> {
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    let started_at: DateTime<Utc> = row.try_get("started_at").map_err(repo_err)?;
    let completed_at: Option<DateTime<Utc>> = row.try_get("completed_at").map_err(repo_err)?;
    Ok(PostProcessingScriptRun {
        id: row.try_get("id").map_err(repo_err)?,
        script_id: row.try_get("script_id").map_err(repo_err)?,
        script_name: row.try_get("script_name").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        title_name: row.try_get("title_name").map_err(repo_err)?,
        facet: row.try_get("facet").map_err(repo_err)?,
        file_path: row.try_get("file_path").map_err(repo_err)?,
        status: scryer_domain::ScriptRunStatus::parse(&status_raw)
            .unwrap_or(scryer_domain::ScriptRunStatus::Failed),
        exit_code: row.try_get("exit_code").map_err(repo_err)?,
        stdout_tail: row.try_get("stdout_tail").map_err(repo_err)?,
        stderr_tail: row.try_get("stderr_tail").map_err(repo_err)?,
        duration_ms: row.try_get("duration_ms").map_err(repo_err)?,
        env_payload_json: row.try_get("env_payload_json").map_err(repo_err)?,
        started_at: started_at.to_rfc3339(),
        completed_at: completed_at.map(|value| value.to_rfc3339()),
    })
}

fn row_to_plugin_installation(row: &sqlx::postgres::PgRow) -> AppResult<PluginInstallation> {
    let source_kind_raw: String = row.try_get("source_kind").map_err(repo_err)?;
    let wasm_encoding_raw: String = row.try_get("wasm_encoding").map_err(repo_err)?;
    let support_tier_raw: String = row.try_get("support_tier").map_err(repo_err)?;
    let descriptor_json: Option<Value> = row.try_get("descriptor_json").map_err(repo_err)?;
    Ok(PluginInstallation {
        id: row.try_get("id").map_err(repo_err)?,
        plugin_id: row.try_get("plugin_id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        description: row.try_get("description").map_err(repo_err)?,
        version: row.try_get("version").map_err(repo_err)?,
        sdk_version: row.try_get("sdk_version").map_err(repo_err)?,
        sdk_constraint: row.try_get("sdk_constraint").map_err(repo_err)?,
        scryer_constraint: row.try_get("scryer_constraint").map_err(repo_err)?,
        plugin_type: row.try_get("plugin_type").map_err(repo_err)?,
        provider_type: row.try_get("provider_type").map_err(repo_err)?,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        is_builtin: row.try_get("is_builtin").map_err(repo_err)?,
        source_kind: parse_source_kind(&source_kind_raw),
        wasm_encoding: parse_wasm_encoding(&wasm_encoding_raw),
        wasm_digest_algo: row.try_get("wasm_digest_algo").map_err(repo_err)?,
        source_url: row.try_get("source_url").map_err(repo_err)?,
        support_tier: parse_support_tier(&support_tier_raw),
        publisher: row.try_get("publisher").map_err(repo_err)?,
        docs_url: row.try_get("docs_url").map_err(repo_err)?,
        source_repo: row.try_get("source_repo").map_err(repo_err)?,
        manifest_url: row.try_get("manifest_url").map_err(repo_err)?,
        wasm_digest: row.try_get("wasm_digest").map_err(repo_err)?,
        artifact_digest: row.try_get("artifact_digest").map_err(repo_err)?,
        descriptor_json: descriptor_json.map(|value| value.to_string()),
        installed_at: row.try_get("installed_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn row_to_plugin_catalog_source(row: &sqlx::postgres::PgRow) -> AppResult<PluginCatalogSource> {
    let support_tier_raw: String = row.try_get("support_tier").map_err(repo_err)?;
    Ok(PluginCatalogSource {
        source_key: row.try_get("source_key").map_err(repo_err)?,
        source_kind: row.try_get("source_kind").map_err(repo_err)?,
        source_url: row.try_get("source_url").map_err(repo_err)?,
        github_repo: row.try_get("github_repo").map_err(repo_err)?,
        support_tier: parse_support_tier(&support_tier_raw),
        catalog_json: row.try_get("catalog_json").map_err(repo_err)?,
        last_success_at: row.try_get("last_success_at").map_err(repo_err)?,
        last_error: row.try_get("last_error").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn row_to_plugin_catalog_status(
    row: &sqlx::postgres::PgRow,
) -> AppResult<PluginCatalogStatusRecord> {
    Ok(PluginCatalogStatusRecord {
        status_key: row.try_get("status_key").map_err(repo_err)?,
        status_json: row.try_get("status_json").map_err(repo_err)?,
        checked_at: row.try_get("checked_at").map_err(repo_err)?,
    })
}

fn optional_json_value(raw: Option<&str>) -> AppResult<Option<Value>> {
    raw.map(json_value).transpose()
}

fn json_value(raw: &str) -> AppResult<Value> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::Validation(format!("invalid logical JSON value: {error}")))
}

fn descriptor_json_is_supported(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        _ => true,
    }
}

fn row_is_incompatible_external_installation(row: &sqlx::postgres::PgRow) -> bool {
    let is_builtin: bool = row.try_get("is_builtin").unwrap_or(false);
    if is_builtin {
        return false;
    }

    let source_kind: String = row
        .try_get("source_kind")
        .unwrap_or_else(|_| "downloaded".to_string());
    if !matches!(source_kind.as_str(), "downloaded" | "manual") {
        return false;
    }

    let wasm_bytes: Option<Vec<u8>> = row.try_get("wasm_bytes").unwrap_or(None);
    let wasm_encoding: String = row
        .try_get("wasm_encoding")
        .unwrap_or_else(|_| "identity".to_string());
    let wasm_digest_algo: Option<String> = row.try_get("wasm_digest_algo").unwrap_or(None);
    let wasm_digest: Option<String> = row.try_get("wasm_digest").unwrap_or(None);
    let descriptor_json: Option<Value> = row.try_get("descriptor_json").unwrap_or(None);

    !external_plugin_installation_is_supported_shape(
        wasm_bytes.as_deref(),
        &wasm_encoding,
        wasm_digest_algo.as_deref(),
        wasm_digest.as_deref(),
        descriptor_json.is_some_and(|value| descriptor_json_is_supported(&value)),
    )
}

fn parse_source_kind(value: &str) -> PluginSourceKind {
    match value {
        "bundled" => PluginSourceKind::Bundled,
        "manual" => PluginSourceKind::Manual,
        _ => PluginSourceKind::Downloaded,
    }
}

fn source_kind_label(value: PluginSourceKind) -> &'static str {
    match value {
        PluginSourceKind::Bundled => "bundled",
        PluginSourceKind::Downloaded => "downloaded",
        PluginSourceKind::Manual => "manual",
    }
}

fn parse_support_tier(value: &str) -> PluginSupportTier {
    match value {
        "verified_community" => PluginSupportTier::VerifiedCommunity,
        "unverified" => PluginSupportTier::Unverified,
        _ => PluginSupportTier::Official,
    }
}

fn support_tier_label(value: PluginSupportTier) -> &'static str {
    match value {
        PluginSupportTier::Official => "official",
        PluginSupportTier::VerifiedCommunity => "verified_community",
        PluginSupportTier::Unverified => "unverified",
    }
}

fn parse_wasm_encoding(value: &str) -> PluginWasmEncoding {
    match value {
        "zstd" => PluginWasmEncoding::Zstd,
        _ => PluginWasmEncoding::Identity,
    }
}

fn wasm_encoding_label(value: PluginWasmEncoding) -> &'static str {
    match value {
        PluginWasmEncoding::Identity => "identity",
        PluginWasmEncoding::Zstd => "zstd",
    }
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}
