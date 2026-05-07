use scryer_application::AppResult;
use scryer_domain::{
    PersistedPluginWasmPayload, PluginCatalogSource, PluginCatalogStatusRecord, PluginInstallation,
    PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
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
        wasm_encoding: parse_wasm_encoding(&row.get::<String, _>("wasm_encoding")),
        wasm_digest_algo: row.get("wasm_digest_algo"),
        source_url: row.get("source_url"),
        support_tier: parse_support_tier(&row.get::<String, _>("support_tier")),
        publisher: row.get("publisher"),
        docs_url: row.get("docs_url"),
        source_repo: row.get("source_repo"),
        manifest_url: row.get("manifest_url"),
        wasm_digest: row.get("wasm_digest"),
        artifact_digest: row.get("artifact_digest"),
        descriptor_json: row.get("descriptor_json"),
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
                source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
                installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_type != '__cache'
         ORDER BY is_builtin DESC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(rows
        .iter()
        .filter(|row| !row_is_incompatible_external_installation(row))
        .map(row_to_plugin_installation)
        .collect())
}

pub(crate) async fn get_plugin_installation_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<Option<PluginInstallation>> {
    let row = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
                installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row
        .as_ref()
        .filter(|row| !row_is_incompatible_external_installation(row))
        .map(row_to_plugin_installation))
}

async fn get_plugin_installation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_id: &str,
) -> AppResult<Option<PluginInstallation>> {
    let row = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
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
    if existing_installation_is_incompatible_external_shape(&mut tx, &installation.plugin_id)
        .await?
    {
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
            .bind(&installation.plugin_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    }
    sqlx::query(
        "INSERT INTO plugin_installations
            (id, plugin_id, name, description, version, sdk_version, sdk_constraint,
             scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
             source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
             docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
             installed_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&installation.descriptor_json)
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

async fn existing_installation_is_incompatible_external_shape(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_id: &str,
) -> AppResult<bool> {
    let row = sqlx::query(
        "SELECT is_builtin, source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, wasm_digest, descriptor_json
         FROM plugin_installations
         WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row
        .as_ref()
        .is_some_and(row_is_incompatible_external_installation))
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
             wasm_encoding = CASE WHEN ? = 'bundled' THEN 'identity' ELSE COALESCE(?, wasm_encoding) END,
             wasm_digest_algo = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, wasm_digest_algo) END,
             source_url = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, source_url) END,
             support_tier = ?,
             publisher = CASE WHEN ? = 'bundled' THEN NULL ELSE ? END,
             docs_url = CASE WHEN ? = 'bundled' THEN NULL ELSE ? END,
             source_repo = CASE WHEN ? = 'bundled' THEN NULL ELSE ? END,
             manifest_url = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, manifest_url) END,
             wasm_digest = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, wasm_digest) END,
             artifact_digest = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, artifact_digest) END,
             descriptor_json = CASE WHEN ? = 'bundled' THEN NULL ELSE COALESCE(?, descriptor_json) END,
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
    .bind(Some(wasm_encoding_label(installation.wasm_encoding)))
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.wasm_digest_algo)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.source_url)
    .bind(support_tier_label(installation.support_tier))
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.publisher)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.docs_url)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.source_repo)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.manifest_url)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.wasm_digest)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.artifact_digest)
    .bind(source_kind_label(installation.source_kind))
    .bind(&installation.descriptor_json)
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

pub(crate) async fn delete_incompatible_external_plugin_installations_query(
    pool: &SqlitePool,
) -> AppResult<Vec<String>> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT plugin_id, wasm_bytes, wasm_encoding, wasm_digest_algo, wasm_digest, descriptor_json
         FROM plugin_installations
         WHERE is_builtin = 0 AND source_kind IN ('downloaded', 'manual')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    let removed_plugin_ids = rows
        .iter()
        .filter_map(|row| {
            let plugin_id: String = row.get("plugin_id");
            let wasm_bytes: Option<Vec<u8>> = row.get("wasm_bytes");
            let wasm_encoding: String = row.get("wasm_encoding");
            let wasm_digest_algo: Option<String> = row.get("wasm_digest_algo");
            let wasm_digest: Option<String> = row.get("wasm_digest");
            let descriptor_json: Option<String> = row.get("descriptor_json");
            if external_installation_is_supported_shape(
                wasm_bytes.as_deref(),
                &wasm_encoding,
                wasm_digest_algo.as_deref(),
                wasm_digest.as_deref(),
                descriptor_json.as_deref(),
            ) {
                None
            } else {
                Some(plugin_id)
            }
        })
        .collect::<Vec<_>>();

    for plugin_id in &removed_plugin_ids {
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(pool)
            .await
            .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    }

    Ok(removed_plugin_ids)
}

pub(crate) async fn get_enabled_plugin_wasm_bytes_query(
    pool: &SqlitePool,
) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, plugin_id, name, description, version, sdk_version, sdk_constraint,
                scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
                source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
                docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
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
            let payload = wasm_bytes.map(|bytes| PersistedPluginWasmPayload {
                encoding: installation.wasm_encoding,
                bytes,
            });
            (installation, payload)
        })
        .collect())
}

pub(crate) async fn get_plugin_installation_wasm_payload_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<Option<PersistedPluginWasmPayload>> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT wasm_bytes, wasm_encoding
         FROM plugin_installations
         WHERE plugin_id = ? AND plugin_type != '__cache'",
    )
    .bind(plugin_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.and_then(|row| {
        let bytes: Option<Vec<u8>> = row.get("wasm_bytes");
        bytes.map(|bytes| PersistedPluginWasmPayload {
            encoding: parse_wasm_encoding(row.get("wasm_encoding")),
            bytes,
        })
    }))
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

fn external_installation_is_supported_shape(
    wasm_bytes: Option<&[u8]>,
    wasm_encoding: &str,
    wasm_digest_algo: Option<&str>,
    wasm_digest: Option<&str>,
    descriptor_json: Option<&str>,
) -> bool {
    wasm_bytes.is_some()
        && wasm_encoding == "zstd"
        && matches!(
            wasm_digest_algo.map(|value| value.trim().to_ascii_lowercase()),
            Some(value) if value == "blake3"
        )
        && wasm_digest.is_some_and(is_hex_digest)
        && descriptor_json.is_some_and(|value| !value.trim().is_empty())
}

fn row_is_incompatible_external_installation(row: &sqlx::sqlite::SqliteRow) -> bool {
    use sqlx::Row;

    let is_builtin: bool = row.get("is_builtin");
    if is_builtin {
        return false;
    }

    let source_kind: String = row.get("source_kind");
    if !matches!(source_kind.as_str(), "downloaded" | "manual") {
        return false;
    }

    let wasm_bytes: Option<Vec<u8>> = row.get("wasm_bytes");
    let wasm_encoding: String = row.get("wasm_encoding");
    let wasm_digest_algo: Option<String> = row.get("wasm_digest_algo");
    let wasm_digest: Option<String> = row.get("wasm_digest");
    let descriptor_json: Option<String> = row.get("descriptor_json");

    !external_installation_is_supported_shape(
        wasm_bytes.as_deref(),
        &wasm_encoding,
        wasm_digest_algo.as_deref(),
        wasm_digest.as_deref(),
        descriptor_json.as_deref(),
    )
}

fn is_hex_digest(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_hexdigit())
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
