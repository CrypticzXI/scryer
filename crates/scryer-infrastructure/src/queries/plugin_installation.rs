use scryer_application::AppResult;
use scryer_domain::{
    PluginCatalogSource, PluginCatalogStatusRecord, PluginInstallation, PluginSourceKind,
    PluginSupportTier,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

#[derive(Clone, Debug)]
pub(crate) struct BuiltinPluginSeed {
    pub plugin_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub sdk_version: String,
    pub sdk_constraint: String,
    pub plugin_type: String,
    pub provider_type: String,
}

fn parse_source_kind(value: &str) -> PluginSourceKind {
    match value {
        "bundled" => PluginSourceKind::Bundled,
        "downloaded" => PluginSourceKind::Downloaded,
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
        "official" => PluginSupportTier::Official,
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

fn row_to_plugin_installation(row: &sqlx::sqlite::SqliteRow) -> PluginInstallation {
    use chrono::{DateTime, Utc};
    use sqlx::Row;

    let installed_str: String = row.get("installed_at");
    let updated_str: String = row.get("updated_at");

    let installed_at = DateTime::parse_from_rfc3339(&installed_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    PluginInstallation {
        id: row.get("id"),
        plugin_id: row.get("plugin_id"),
        name: row.get("name"),
        description: row.get("description"),
        version: row.get("version"),
        sdk_version: row.get("sdk_version"),
        sdk_constraint: row.get("sdk_constraint"),
        scryer_constraint: row.get("scryer_constraint"),
        plugin_type: row.get("plugin_type"),
        provider_type: row.get("provider_type"),
        is_enabled: row.get::<i32, _>("is_enabled") != 0,
        is_builtin: row.get::<i32, _>("is_builtin") != 0,
        source_kind: parse_source_kind(&row.get::<String, _>("source_kind")),
        wasm_sha256: row.get("wasm_sha256"),
        source_url: row.get("source_url"),
        support_tier: parse_support_tier(&row.get::<String, _>("support_tier")),
        publisher: row.get("publisher"),
        docs_url: row.get("docs_url"),
        source_repo: row.get("source_repo"),
        manifest_url: row.get("manifest_url"),
        wasm_digest: row.get("wasm_digest"),
        artifact_digest: row.get("artifact_digest"),
        installed_at,
        updated_at,
    }
}

pub(crate) async fn list_plugin_installations_query(
    pool: &SqlitePool,
) -> AppResult<Vec<PluginInstallation>> {
    let rows = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_sha256, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
                installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_type != '__cache'
         ORDER BY is_builtin DESC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(rows.iter().map(row_to_plugin_installation).collect())
}

pub(crate) async fn get_plugin_installation_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<Option<PluginInstallation>> {
    let row = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_sha256, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
                installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.as_ref().map(row_to_plugin_installation))
}

async fn get_plugin_installation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_id: &str,
) -> AppResult<Option<PluginInstallation>> {
    let row = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_sha256, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
                installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.as_ref().map(row_to_plugin_installation))
}

pub(crate) async fn create_plugin_installation_query(
    pool: &SqlitePool,
    installation: &PluginInstallation,
    wasm_bytes: Option<&[u8]>,
) -> AppResult<PluginInstallation> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    sqlx::query(
        "INSERT INTO plugin_installations
            (id, plugin_id, name, description, version, sdk_version, sdk_constraint,
             scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
             source_kind, wasm_bytes, wasm_sha256, source_url, support_tier, publisher,
             docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
             installed_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(installation.is_enabled as i32)
    .bind(installation.is_builtin as i32)
    .bind(source_kind_label(installation.source_kind))
    .bind(wasm_bytes)
    .bind(&installation.wasm_sha256)
    .bind(&installation.source_url)
    .bind(support_tier_label(installation.support_tier))
    .bind(&installation.publisher)
    .bind(&installation.docs_url)
    .bind(&installation.source_repo)
    .bind(&installation.manifest_url)
    .bind(&installation.wasm_digest)
    .bind(&installation.artifact_digest)
    .bind(installation.installed_at.to_rfc3339())
    .bind(installation.updated_at.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    let installation = get_plugin_installation_tx(&mut tx, &installation.plugin_id)
        .await?
        .ok_or_else(|| {
            scryer_application::AppError::Repository(
                "failed to read back created plugin installation".to_string(),
            )
        })?;
    tx.commit()
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(installation)
}

pub(crate) async fn update_plugin_installation_query(
    pool: &SqlitePool,
    installation: &PluginInstallation,
    wasm_bytes: Option<&[u8]>,
) -> AppResult<PluginInstallation> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    sqlx::query(
        "UPDATE plugin_installations
         SET name = ?, description = ?, version = ?, sdk_version = ?, sdk_constraint = ?,
             scryer_constraint = ?, plugin_type = ?, provider_type = ?, is_enabled = ?,
             is_builtin = ?, source_kind = ?,
             wasm_bytes = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, wasm_bytes) END,
             wasm_sha256 = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, wasm_sha256) END,
             source_url = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, source_url) END,
             support_tier = ?, publisher = ?, docs_url = ?, source_repo = ?,
             manifest_url = ?, wasm_digest = ?, artifact_digest = ?,
             updated_at = ?
         WHERE plugin_id = ?",
    )
    .bind(&installation.name)
    .bind(&installation.description)
    .bind(&installation.version)
    .bind(&installation.sdk_version)
    .bind(&installation.sdk_constraint)
    .bind(&installation.scryer_constraint)
    .bind(&installation.plugin_type)
    .bind(&installation.provider_type)
    .bind(installation.is_enabled as i32)
    .bind(installation.is_builtin as i32)
    .bind(source_kind_label(installation.source_kind))
    .bind(source_kind_label(installation.source_kind))
    .bind(wasm_bytes)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.wasm_sha256)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.source_url)
    .bind(support_tier_label(installation.support_tier))
    .bind(&installation.publisher)
    .bind(&installation.docs_url)
    .bind(&installation.source_repo)
    .bind(&installation.manifest_url)
    .bind(&installation.wasm_digest)
    .bind(&installation.artifact_digest)
    .bind(installation.updated_at.to_rfc3339())
    .bind(&installation.plugin_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    let installation = get_plugin_installation_tx(&mut tx, &installation.plugin_id)
        .await?
        .ok_or_else(|| {
            scryer_application::AppError::Repository(
                "failed to read back updated plugin installation".to_string(),
            )
        })?;
    tx.commit()
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(installation)
}

pub(crate) async fn delete_plugin_installation_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
        .bind(plugin_id)
        .execute(pool)
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_enabled_plugin_wasm_bytes_query(
    pool: &SqlitePool,
) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_bytes, wasm_sha256, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
                installed_at, updated_at
         FROM plugin_installations
         WHERE is_enabled = 1 AND plugin_type != '__cache'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|row| {
            let installation = row_to_plugin_installation(row);
            let wasm_bytes: Option<Vec<u8>> = row.get("wasm_bytes");
            (installation, wasm_bytes)
        })
        .collect())
}

pub(crate) async fn seed_builtin_query(
    pool: &SqlitePool,
    seed: &BuiltinPluginSeed,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = scryer_domain::Id::new().0;
    sqlx::query(
        "INSERT OR IGNORE INTO plugin_installations
            (id, plugin_id, name, description, version, sdk_version, sdk_constraint,
             scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
             source_kind, installed_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, 1, 1, 'bundled', ?, ?)",
    )
    .bind(&id)
    .bind(&seed.plugin_id)
    .bind(&seed.name)
    .bind(&seed.description)
    .bind(&seed.version)
    .bind(&seed.sdk_version)
    .bind(&seed.sdk_constraint)
    .bind(&seed.plugin_type)
    .bind(&seed.provider_type)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    sqlx::query(
        "UPDATE plugin_installations
         SET name = CASE WHEN source_kind = 'downloaded' THEN name ELSE ? END,
             description = CASE WHEN source_kind = 'downloaded' THEN description ELSE ? END,
             version = CASE WHEN source_kind = 'downloaded' THEN version ELSE ? END,
             sdk_version = CASE WHEN source_kind = 'downloaded' THEN sdk_version ELSE ? END,
             sdk_constraint = CASE WHEN source_kind = 'downloaded' THEN sdk_constraint ELSE ? END,
             scryer_constraint = CASE WHEN source_kind = 'downloaded' THEN scryer_constraint ELSE NULL END,
             plugin_type = ?,
             provider_type = ?,
             source_kind = CASE WHEN source_kind = 'downloaded' THEN source_kind ELSE 'bundled' END,
             updated_at = ?
         WHERE plugin_id = ? AND is_builtin = 1",
    )
    .bind(&seed.name)
    .bind(&seed.description)
    .bind(&seed.version)
    .bind(&seed.sdk_version)
    .bind(&seed.sdk_constraint)
    .bind(&seed.plugin_type)
    .bind(&seed.provider_type)
    .bind(&now)
    .bind(&seed.plugin_id)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

fn parse_optional_datetime(value: Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
    value.and_then(|raw| {
        chrono::DateTime::parse_from_rfc3339(&raw)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok()
    })
}

fn row_to_plugin_catalog_source(row: &sqlx::sqlite::SqliteRow) -> PluginCatalogSource {
    use sqlx::Row;

    let updated_at: String = row.get("updated_at");
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());

    PluginCatalogSource {
        source_key: row.get("source_key"),
        source_kind: row.get("source_kind"),
        source_url: row.get("source_url"),
        github_repo: row.get("github_repo"),
        support_tier: parse_support_tier(&row.get::<String, _>("support_tier")),
        catalog_json: row.get("catalog_json"),
        last_success_at: parse_optional_datetime(row.get("last_success_at")),
        last_error: row.get("last_error"),
        updated_at,
    }
}

pub(crate) async fn upsert_plugin_catalog_source_query(
    pool: &SqlitePool,
    source: &PluginCatalogSource,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO plugin_catalog_sources
            (source_key, source_kind, source_url, github_repo, support_tier, catalog_json,
             last_success_at, last_error, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(source_key) DO UPDATE SET
             source_kind = excluded.source_kind,
             source_url = excluded.source_url,
             github_repo = excluded.github_repo,
             support_tier = excluded.support_tier,
             catalog_json = excluded.catalog_json,
             last_success_at = excluded.last_success_at,
             last_error = excluded.last_error,
             updated_at = excluded.updated_at",
    )
    .bind(&source.source_key)
    .bind(&source.source_kind)
    .bind(&source.source_url)
    .bind(&source.github_repo)
    .bind(support_tier_label(source.support_tier))
    .bind(&source.catalog_json)
    .bind(source.last_success_at.map(|dt| dt.to_rfc3339()))
    .bind(&source.last_error)
    .bind(source.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn list_plugin_catalog_sources_query(
    pool: &SqlitePool,
) -> AppResult<Vec<PluginCatalogSource>> {
    let rows = sqlx::query(
        "SELECT source_key, source_kind, source_url, github_repo, support_tier, catalog_json,
                last_success_at, last_error, updated_at
         FROM plugin_catalog_sources
         ORDER BY source_kind ASC, source_key ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(rows.iter().map(row_to_plugin_catalog_source).collect())
}

pub(crate) async fn get_plugin_catalog_source_query(
    pool: &SqlitePool,
    source_key: &str,
) -> AppResult<Option<PluginCatalogSource>> {
    let row = sqlx::query(
        "SELECT source_key, source_kind, source_url, github_repo, support_tier, catalog_json,
                last_success_at, last_error, updated_at
         FROM plugin_catalog_sources
         WHERE source_key = ?",
    )
    .bind(source_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(row.as_ref().map(row_to_plugin_catalog_source))
}

fn row_to_plugin_catalog_status(row: &sqlx::sqlite::SqliteRow) -> PluginCatalogStatusRecord {
    use sqlx::Row;

    let checked_at: String = row.get("checked_at");
    let checked_at = chrono::DateTime::parse_from_rfc3339(&checked_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());

    PluginCatalogStatusRecord {
        status_key: row.get("status_key"),
        status_json: row.get("status_json"),
        checked_at,
    }
}

pub(crate) async fn upsert_plugin_catalog_status_query(
    pool: &SqlitePool,
    status: &PluginCatalogStatusRecord,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO plugin_catalog_status (status_key, status_json, checked_at)
         VALUES (?, ?, ?)
         ON CONFLICT(status_key) DO UPDATE SET
             status_json = excluded.status_json,
             checked_at = excluded.checked_at",
    )
    .bind(&status.status_key)
    .bind(&status.status_json)
    .bind(status.checked_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_plugin_catalog_status_query(
    pool: &SqlitePool,
    status_key: &str,
) -> AppResult<Option<PluginCatalogStatusRecord>> {
    let row = sqlx::query(
        "SELECT status_key, status_json, checked_at
         FROM plugin_catalog_status
         WHERE status_key = ?",
    )
    .bind(status_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(row.as_ref().map(row_to_plugin_catalog_status))
}

pub(crate) async fn store_registry_cache_query(pool: &SqlitePool, json: &str) -> AppResult<()> {
    // Use a special plugin_id "__registry_cache" to store the JSON in the same table
    // This avoids needing a separate table or expanding the settings system.
    let now = chrono::Utc::now().to_rfc3339();
    let id = scryer_domain::Id::new().0;
    sqlx::query(
        "INSERT INTO plugin_installations
            (id, plugin_id, name, description, version, plugin_type, provider_type,
             is_enabled, is_builtin, wasm_sha256, installed_at, updated_at)
         VALUES (?, '__registry_cache', '__registry_cache', ?, '', '__cache', '__cache', 0, 0, NULL, ?, ?)
         ON CONFLICT(plugin_id) DO UPDATE SET description = excluded.description, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(json)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_registry_cache_query(pool: &SqlitePool) -> AppResult<Option<String>> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT description FROM plugin_installations WHERE plugin_id = '__registry_cache'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.map(|r| r.get::<String, _>("description")))
}
