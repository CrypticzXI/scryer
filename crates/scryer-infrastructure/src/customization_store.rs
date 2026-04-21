use async_trait::async_trait;
use scryer_application::{
    AppResult, PluginInstallationRepository, PostProcessingScriptRepository, RuleSetRepository,
};
use scryer_domain::{PluginInstallation, PostProcessingScript, PostProcessingScriptRun, RuleSet};

use crate::SqliteServices;
use crate::queries::{plugin_installation, post_processing_script, rule_set};

#[derive(Clone)]
pub struct SqliteCustomizationStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteCustomizationStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl RuleSetRepository for SqliteCustomizationStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        rule_set::list_rule_sets_query(&self.pool).await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        rule_set::list_enabled_rule_sets_query(&self.pool).await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        rule_set::get_rule_set_by_id_query(&self.pool, id).await
    }

    async fn create_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()> {
        self.db.create_rule_set(rule_set_record).await
    }

    async fn update_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()> {
        self.db.update_rule_set(rule_set_record).await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.db.delete_rule_set(id).await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        let id = scryer_domain::Id::new().0;
        self.db
            .record_rule_set_history(&id, rule_set_id, action, rego_source, actor_id)
            .await
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        rule_set::get_rule_set_by_managed_key_query(&self.pool, key).await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        self.db.delete_rule_set_by_managed_key(key).await
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        rule_set::list_rule_sets_by_managed_key_prefix_query(&self.pool, prefix).await
    }
}

#[async_trait]
impl PostProcessingScriptRepository for SqliteCustomizationStore {
    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        post_processing_script::list_scripts_query(&self.pool).await
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        post_processing_script::get_script_by_id_query(&self.pool, id).await
    }

    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.db.create_post_processing_script(script).await
    }

    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.db.update_post_processing_script(script).await
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        self.db.delete_post_processing_script(id).await
    }

    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>> {
        post_processing_script::list_enabled_for_facet_query(&self.pool, facet).await
    }

    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        self.db.record_post_processing_script_run(run).await
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        post_processing_script::list_runs_for_script_query(&self.pool, script_id, limit).await
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        post_processing_script::list_runs_for_title_query(&self.pool, title_id, limit).await
    }
}

#[async_trait]
impl PluginInstallationRepository for SqliteCustomizationStore {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        plugin_installation::list_plugin_installations_query(&self.pool).await
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        plugin_installation::get_plugin_installation_query(&self.pool, plugin_id).await
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.db
            .create_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.db
            .update_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        self.db.delete_plugin_installation(plugin_id).await
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
        plugin_installation::get_enabled_plugin_wasm_bytes_query(&self.pool).await
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        self.db
            .seed_builtin_plugin(plugin_id, name, description, version, provider_type)
            .await
    }

    async fn store_registry_cache(&self, json: &str) -> AppResult<()> {
        self.db.store_plugin_registry_cache(json).await
    }

    async fn get_registry_cache(&self) -> AppResult<Option<String>> {
        plugin_installation::get_registry_cache_query(&self.pool).await
    }
}
