use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    Id, PersistedPluginWasmPayload, PluginCatalogSource, PluginCatalogStatusRecord,
    PluginInstallation, PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
    PostProcessingScript, PostProcessingScriptRun, RuleSet,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
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
        let rows = sqlx::query(
            "SELECT plugin_id, wasm_bytes, wasm_encoding, wasm_digest_algo, wasm_digest, descriptor_json
             FROM plugin_installations
             WHERE is_builtin = FALSE AND source_kind IN ('downloaded', 'manual')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        let mut removed_plugin_ids = Vec::new();
        for row in rows {
            let plugin_id: String = row.try_get("plugin_id").map_err(repo_err)?;
            let wasm_bytes: Option<Vec<u8>> = row.try_get("wasm_bytes").map_err(repo_err)?;
            let wasm_encoding: String = row.try_get("wasm_encoding").map_err(repo_err)?;
            let wasm_digest_algo: Option<String> =
                row.try_get("wasm_digest_algo").map_err(repo_err)?;
            let wasm_digest: Option<String> = row.try_get("wasm_digest").map_err(repo_err)?;
            let descriptor_json: Option<Value> =
                row.try_get("descriptor_json").map_err(repo_err)?;

            if !external_installation_is_supported_shape(
                wasm_bytes.as_deref(),
                &wasm_encoding,
                wasm_digest_algo.as_deref(),
                wasm_digest.as_deref(),
                descriptor_json.as_ref(),
            ) {
                removed_plugin_ids.push(plugin_id);
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
        let record_json = serde_json::to_value(installation).map_err(repo_err)?;
        let descriptor_json = optional_json_value(installation.descriptor_json.as_deref())?;
        sqlx::query(
            "INSERT INTO plugin_installations (
                id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, source_kind, is_enabled,
                is_builtin, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url,
                support_tier, publisher, docs_url, source_repo, manifest_url, wasm_digest,
                artifact_digest, descriptor_json, record_json, installed_at, updated_at
             )
             VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17,
                $18, $19, $20, $21, $22, $23,
                $24, $25::jsonb, $26::jsonb, $27, $28
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
                record_json = EXCLUDED.record_json,
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
        .bind(enum_json_string(&installation.source_kind)?)
        .bind(installation.is_enabled)
        .bind(installation.is_builtin)
        .bind(wasm_bytes.map(|bytes| bytes.to_vec()))
        .bind(enum_json_string(&installation.wasm_encoding)?)
        .bind(&installation.wasm_digest_algo)
        .bind(&installation.source_url)
        .bind(enum_json_string(&installation.support_tier)?)
        .bind(&installation.publisher)
        .bind(&installation.docs_url)
        .bind(&installation.source_repo)
        .bind(&installation.manifest_url)
        .bind(&installation.wasm_digest)
        .bind(&installation.artifact_digest)
        .bind(descriptor_json)
        .bind(record_json)
        .bind(installation.installed_at)
        .bind(installation.updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(installation.clone())
    }

    async fn upsert_post_processing_script(&self, script: &PostProcessingScript) -> AppResult<()> {
        let record_json = serde_json::to_value(script).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO post_processing_scripts
             (id, name, script_path, is_enabled, created_at, updated_at, record_json, priority)
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                script_path = EXCLUDED.script_path,
                is_enabled = EXCLUDED.is_enabled,
                updated_at = EXCLUDED.updated_at,
                record_json = EXCLUDED.record_json,
                priority = EXCLUDED.priority",
        )
        .bind(&script.id)
        .bind(&script.name)
        .bind(&script.script_content)
        .bind(script.enabled)
        .bind(script.created_at)
        .bind(script.updated_at)
        .bind(record_json)
        .bind(script.priority)
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
        let rows =
            sqlx::query("SELECT record_json FROM rule_sets ORDER BY priority DESC, name ASC")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let rows = sqlx::query(
            "SELECT record_json FROM rule_sets WHERE enabled = TRUE ORDER BY priority DESC, name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let row = sqlx::query("SELECT record_json FROM rule_sets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }
    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        self.update_rule_set(rule_set).await
    }
    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let record_json = serde_json::to_value(rule_set).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO rule_sets
             (id, name, managed_key, rule_json, record_json, enabled, priority, created_at, updated_at, is_managed)
             VALUES ($1, $2, $3, $4::jsonb, $4::jsonb, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                managed_key = EXCLUDED.managed_key,
                rule_json = EXCLUDED.rule_json,
                record_json = EXCLUDED.record_json,
                enabled = EXCLUDED.enabled,
                priority = EXCLUDED.priority,
                updated_at = EXCLUDED.updated_at,
                is_managed = EXCLUDED.is_managed",
        )
        .bind(&rule_set.id)
        .bind(&rule_set.name)
        .bind(&rule_set.managed_key)
        .bind(record_json)
        .bind(rule_set.enabled)
        .bind(rule_set.priority)
        .bind(rule_set.created_at)
        .bind(rule_set.updated_at)
        .bind(rule_set.is_managed)
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
        let row = sqlx::query("SELECT record_json FROM rule_sets WHERE managed_key = $1 LIMIT 1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
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
        let rows = sqlx::query(
            "SELECT record_json FROM rule_sets WHERE managed_key LIKE $1 ORDER BY managed_key",
        )
        .bind(pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        let rows = sqlx::query(
            "SELECT record_json FROM post_processing_scripts ORDER BY priority DESC, name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        let row = sqlx::query("SELECT record_json FROM post_processing_scripts WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
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
        let rows = sqlx::query(
            "SELECT record_json
               FROM post_processing_scripts
              WHERE is_enabled = TRUE
                AND (record_json->'applied_facets' = '[]'::jsonb OR record_json->'applied_facets' ? $1)
              ORDER BY priority DESC, name ASC",
        )
        .bind(facet)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        let record_json = serde_json::to_value(&run).map_err(repo_err)?;
        let started_at =
            parse_rfc3339_timestamp(&run.started_at, "post_processing_script_runs.started_at")?;
        let completed_at = parse_optional_rfc3339_timestamp(
            run.completed_at.as_deref(),
            "post_processing_script_runs.finished_at",
        )?;
        sqlx::query(
            "INSERT INTO post_processing_script_runs
             (id, script_id, status, output_text, started_at, finished_at, created_at, record_json)
             VALUES ($1, $2, $3, $4, $5::timestamptz, $6::timestamptz, NOW(), $7::jsonb)
             ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                output_text = EXCLUDED.output_text,
                finished_at = EXCLUDED.finished_at,
                record_json = EXCLUDED.record_json",
        )
        .bind(&run.id)
        .bind(&run.script_id)
        .bind(run.status.as_str())
        .bind(run.stderr_tail.as_deref().or(run.stdout_tail.as_deref()))
        .bind(started_at)
        .bind(completed_at)
        .bind(record_json)
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
        let rows = sqlx::query(
            "SELECT record_json FROM post_processing_script_runs
              WHERE script_id = $1
              ORDER BY started_at DESC
              LIMIT $2",
        )
        .bind(script_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let rows = sqlx::query(
            "SELECT record_json FROM post_processing_script_runs
              WHERE record_json->>'title_id' = $1
              ORDER BY started_at DESC
              LIMIT $2",
        )
        .bind(title_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        let rows = sqlx::query(
            "SELECT record_json FROM plugin_installations ORDER BY is_builtin DESC, name, plugin_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let row = sqlx::query("SELECT record_json FROM plugin_installations WHERE plugin_id = $1")
            .bind(plugin_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
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
        let rows = sqlx::query(
            "SELECT record_json, wasm_bytes
               FROM plugin_installations
              WHERE is_enabled = TRUE
              ORDER BY is_builtin DESC, name, plugin_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        rows.iter()
            .map(|row| {
                let installation: PluginInstallation = record_from_row(row)?;
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
            "SELECT record_json, wasm_bytes FROM plugin_installations WHERE plugin_id = $1",
        )
        .bind(plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        let Some(row) = row else {
            return Ok(None);
        };
        let installation: PluginInstallation = record_from_row(&row)?;
        let bytes: Option<Vec<u8>> = row.try_get("wasm_bytes").map_err(repo_err)?;
        Ok(bytes.map(|bytes| PersistedPluginWasmPayload {
            encoding: installation.wasm_encoding,
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
        let record_json = serde_json::to_value(source).map_err(repo_err)?;
        let catalog_json = optional_json_value(source.catalog_json.as_deref())?;
        sqlx::query(
            "INSERT INTO plugin_catalog_sources (
                source_key, source_kind, source_url, github_repo, support_tier,
                catalog_json, record_json, last_success_at, last_error, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8, $9, $10)
             ON CONFLICT (source_key) DO UPDATE SET
                source_kind = EXCLUDED.source_kind,
                source_url = EXCLUDED.source_url,
                github_repo = EXCLUDED.github_repo,
                support_tier = EXCLUDED.support_tier,
                catalog_json = EXCLUDED.catalog_json,
                record_json = EXCLUDED.record_json,
                last_success_at = EXCLUDED.last_success_at,
                last_error = EXCLUDED.last_error,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&source.source_key)
        .bind(&source.source_kind)
        .bind(&source.source_url)
        .bind(&source.github_repo)
        .bind(enum_json_string(&source.support_tier)?)
        .bind(catalog_json)
        .bind(record_json)
        .bind(source.last_success_at)
        .bind(&source.last_error)
        .bind(source.updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>> {
        let rows =
            sqlx::query("SELECT record_json FROM plugin_catalog_sources ORDER BY source_key")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        rows.iter().map(record_from_row).collect()
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>> {
        let row =
            sqlx::query("SELECT record_json FROM plugin_catalog_sources WHERE source_key = $1")
                .bind(source_key)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        let record_json = serde_json::to_value(status).map_err(repo_err)?;
        let status_json = json_value(&status.status_json)?;
        sqlx::query(
            "INSERT INTO plugin_catalog_status (
                status_key, catalog_json, record_json, last_success_at, updated_at
             )
             VALUES ($1, $2::jsonb, $3::jsonb, $4, $5)
             ON CONFLICT (status_key) DO UPDATE SET
                catalog_json = EXCLUDED.catalog_json,
                record_json = EXCLUDED.record_json,
                last_success_at = EXCLUDED.last_success_at,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&status.status_key)
        .bind(status_json)
        .bind(record_json)
        .bind(status.checked_at)
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
        let row =
            sqlx::query("SELECT record_json FROM plugin_catalog_status WHERE status_key = $1")
                .bind(status_key)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }
}

fn record_from_row<T: DeserializeOwned>(row: &sqlx::postgres::PgRow) -> AppResult<T> {
    let value: Value = row.try_get("record_json").map_err(repo_err)?;
    serde_json::from_value(value).map_err(repo_err)
}

fn optional_json_value(raw: Option<&str>) -> AppResult<Option<Value>> {
    raw.map(json_value).transpose()
}

fn json_value(raw: &str) -> AppResult<Value> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::Validation(format!("invalid logical JSON value: {error}")))
}

fn external_installation_is_supported_shape(
    wasm_bytes: Option<&[u8]>,
    wasm_encoding: &str,
    wasm_digest_algo: Option<&str>,
    wasm_digest: Option<&str>,
    descriptor_json: Option<&Value>,
) -> bool {
    wasm_bytes.is_some()
        && wasm_encoding == "zstd"
        && matches!(
            wasm_digest_algo.map(|value| value.trim().to_ascii_lowercase()),
            Some(value) if value == "blake3"
        )
        && wasm_digest.is_some_and(is_hex_digest)
        && descriptor_json.is_some_and(descriptor_json_is_supported)
}

fn is_hex_digest(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn descriptor_json_is_supported(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        _ => true,
    }
}

fn enum_json_string<T: Serialize>(value: &T) -> AppResult<String> {
    let value = serde_json::to_value(value).map_err(repo_err)?;
    value.as_str().map(str::to_string).ok_or_else(|| {
        AppError::Repository("expected enum to serialize as a JSON string".to_string())
    })
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}
