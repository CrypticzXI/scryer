use async_trait::async_trait;
use scryer_application::{
    AppResult, PluginInstallationRepository, PostProcessingScriptRepository, RuleSetRepository,
};
use scryer_domain::{
    PersistedPluginWasmPayload, PluginCatalogSource, PluginCatalogStatusRecord, PluginInstallation,
    PostProcessingScript, PostProcessingScriptRun, RuleSet,
};

use crate::SqliteServices;
use crate::queries::{
    plugin_installation::{self, BuiltinPluginSeed},
    post_processing_script, rule_set,
};

#[async_trait]
pub trait CustomizationSql: Clone + Send + Sync + 'static {
    async fn delete_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>>;

    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>>;
    async fn create_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()>;
    async fn update_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()>;
    async fn delete_rule_set(&self, id: &str) -> AppResult<()>;
    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()>;
    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>>;
    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()>;
    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>>;

    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>>;
    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>>;
    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript>;
    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript>;
    async fn delete_script(&self, id: &str) -> AppResult<()>;
    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>>;
    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()>;
    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>>;
    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>>;

    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>>;
    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>>;
    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()>;
    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>>;
    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>>;
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
    ) -> AppResult<()>;
    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()>;
    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>>;
    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>>;
    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()>;
    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>>;
}

#[derive(Clone)]
pub struct CustomizationStore<S> {
    sql: S,
}

impl<S> CustomizationStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

impl<S: CustomizationSql> CustomizationStore<S> {
    pub async fn delete_incompatible_external_plugin_installations(
        &self,
    ) -> AppResult<Vec<String>> {
        self.sql
            .delete_incompatible_external_plugin_installations()
            .await
    }
}

pub type SqliteCustomizationStore = CustomizationStore<SqliteCustomizationSql>;

#[derive(Clone)]
pub struct SqliteCustomizationSql {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteCustomizationStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteCustomizationSql::new(db))
    }
}

impl SqliteCustomizationSql {
    fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl CustomizationSql for SqliteCustomizationSql {
    async fn delete_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>> {
        plugin_installation::delete_incompatible_external_plugin_installations_query(&self.pool)
            .await
    }

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
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        plugin_installation::get_enabled_plugin_wasm_bytes_query(&self.pool).await
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        plugin_installation::get_plugin_installation_wasm_payload_query(&self.pool, plugin_id).await
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
        self.db
            .seed_builtin_plugin(BuiltinPluginSeed {
                plugin_id: plugin_id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                version: version.to_string(),
                sdk_version: sdk_version.to_string(),
                sdk_constraint: sdk_constraint.to_string(),
                plugin_type: plugin_type.to_string(),
                provider_type: provider_type.to_string(),
            })
            .await
    }

    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()> {
        plugin_installation::upsert_plugin_catalog_source_query(&self.pool, source).await
    }

    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>> {
        plugin_installation::list_plugin_catalog_sources_query(&self.pool).await
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>> {
        plugin_installation::get_plugin_catalog_source_query(&self.pool, source_key).await
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        plugin_installation::upsert_plugin_catalog_status_query(&self.pool, status).await
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>> {
        plugin_installation::get_plugin_catalog_status_query(&self.pool, status_key).await
    }
}

#[async_trait]
impl<S: CustomizationSql> RuleSetRepository for CustomizationStore<S> {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        self.sql.list_rule_sets().await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        self.sql.list_enabled_rule_sets().await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        self.sql.get_rule_set(id).await
    }

    async fn create_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()> {
        self.sql.create_rule_set(rule_set_record).await
    }

    async fn update_rule_set(&self, rule_set_record: &RuleSet) -> AppResult<()> {
        self.sql.update_rule_set(rule_set_record).await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.sql.delete_rule_set(id).await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        self.sql
            .record_rule_set_history(rule_set_id, action, rego_source, actor_id)
            .await
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        self.sql.get_rule_set_by_managed_key(key).await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        self.sql.delete_rule_set_by_managed_key(key).await
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        self.sql.list_rule_sets_by_managed_key_prefix(prefix).await
    }
}

#[async_trait]
impl<S: CustomizationSql> PostProcessingScriptRepository for CustomizationStore<S> {
    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        self.sql.list_scripts().await
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        self.sql.get_script(id).await
    }

    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.sql.create_script(script).await
    }

    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        self.sql.update_script(script).await
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        self.sql.delete_script(id).await
    }

    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>> {
        self.sql.list_enabled_for_facet(facet).await
    }

    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        self.sql.record_run(run).await
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        self.sql.list_runs_for_script(script_id, limit).await
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        self.sql.list_runs_for_title(title_id, limit).await
    }
}

#[async_trait]
impl<S: CustomizationSql> PluginInstallationRepository for CustomizationStore<S> {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        self.sql.list_plugin_installations().await
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        self.sql.get_plugin_installation(plugin_id).await
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.sql
            .create_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.sql
            .update_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        self.sql.delete_plugin_installation(plugin_id).await
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        self.sql.get_enabled_plugin_wasm_bytes().await
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        self.sql
            .get_plugin_installation_wasm_payload(plugin_id)
            .await
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
        self.sql
            .seed_builtin(
                plugin_id,
                name,
                description,
                version,
                sdk_version,
                sdk_constraint,
                plugin_type,
                provider_type,
            )
            .await
    }

    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()> {
        self.sql.upsert_plugin_catalog_source(source).await
    }

    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>> {
        self.sql.list_plugin_catalog_sources().await
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>> {
        self.sql.get_plugin_catalog_source(source_key).await
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        self.sql.upsert_plugin_catalog_status(status).await
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>> {
        self.sql.get_plugin_catalog_status(status_key).await
    }
}
