use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use scryer_application::AppUseCase;
use scryer_domain::AppPermission;
use scryer_infrastructure::SettingsStore;
use scryer_interface::context::AuthRuntimeStateHandle;
use serde::{Deserialize, Serialize};

use crate::middleware::{map_app_error, resolve_actor_with_app_permission};
use crate::settings_bootstrap::{SETTINGS_SCOPE_MEDIA, SETTINGS_SCOPE_SYSTEM};

pub(crate) async fn bootstrap_admin_password(app_use_case: &AppUseCase) {
    let admin = match app_use_case.find_or_create_default_user().await {
        Ok(user) => user,
        Err(error) => {
            tracing::warn!(error = %error, "failed to look up admin user for password bootstrap");
            return;
        }
    };

    if admin.password_hash.is_some() {
        return;
    }

    match app_use_case
        .bootstrap_user_password(&admin.id, "admin")
        .await
    {
        Ok(_) => tracing::info!("admin password bootstrapped (change it in Settings > Users)"),
        Err(error) => tracing::warn!(error = %error, "failed to set admin password"),
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminMigrationsResponse {
    applied_migrations: Vec<AdminAppliedMigration>,
    pending_migrations: Vec<String>,
    latest_successful_migration_key: Option<String>,
    migration_checksum_mismatch_flags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminAppliedMigration {
    migration_key: String,
    migration_checksum_algo: String,
    migration_checksum: String,
    migration_checksum_algo_expected: Option<String>,
    migration_checksum_expected: Option<String>,
    checksum_mismatch: bool,
    applied_at: String,
    success: bool,
    error_message: Option<String>,
    runtime_version: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminSettingsResponse {
    scope: String,
    scope_id: Option<String>,
    items: Vec<AdminSettingItem>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminSettingItem {
    category: String,
    scope: String,
    key_name: String,
    data_type: String,
    default_value_json: String,
    effective_value_json: Option<String>,
    value_json: Option<String>,
    source: Option<String>,
    has_override: bool,
    is_sensitive: bool,
    validation_json: Option<String>,
    scope_id: Option<String>,
    updated_by_user_id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminSettingsQuery {
    scope: Option<String>,
    scope_id: Option<String>,
    category: Option<String>,
}

fn permission_for_settings_scope(scope: &str) -> AppPermission {
    if scope == SETTINGS_SCOPE_MEDIA {
        AppPermission::ManageCatalogSettings
    } else {
        AppPermission::ManageSystemSettings
    }
}

pub(crate) async fn admin_settings_list(
    database: Arc<SettingsStore>,
    app_use_case: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    headers: HeaderMap,
    remote_addr: SocketAddr,
    query: AdminSettingsQuery,
) -> Response {
    let scope = query
        .scope
        .as_deref()
        .unwrap_or(SETTINGS_SCOPE_SYSTEM)
        .to_string();
    let required_permission = permission_for_settings_scope(&scope);
    let _actor = match resolve_actor_with_app_permission(
        &app_use_case,
        &auth_runtime,
        &headers,
        Some(remote_addr),
        required_permission,
    )
    .await
    {
        Ok(actor) => actor,
        Err(error) => return map_app_error(error),
    };

    let category_filter = query.category.map(|value| value.trim().to_string());

    let records = match database
        .list_settings_with_defaults(&scope, query.scope_id.clone())
        .await
    {
        Ok(records) => records,
        Err(error) => return map_app_error(error),
    };

    let items = records
        .into_iter()
        .filter(|record| {
            category_filter
                .as_deref()
                .is_none_or(|target| record.category == target)
        })
        .map(|record| {
            let has_override = record.has_override();
            let is_sensitive = record.is_sensitive;
            let effective_value_json = if is_sensitive {
                None
            } else {
                Some(record.effective_value_json)
            };
            let value_json = if is_sensitive {
                None
            } else {
                record.value_json
            };

            AdminSettingItem {
                category: record.category,
                scope: record.scope,
                key_name: record.key_name,
                data_type: record.data_type,
                default_value_json: record.default_value_json,
                effective_value_json,
                value_json,
                source: record.source,
                has_override,
                is_sensitive,
                validation_json: record.validation_json,
                scope_id: record.scope_id,
                updated_by_user_id: record.updated_by_user_id,
                created_at: record.created_at,
                updated_at: record.updated_at,
            }
        })
        .collect::<Vec<_>>();

    Json(AdminSettingsResponse {
        scope,
        scope_id: query.scope_id,
        items,
    })
    .into_response()
}

#[derive(Debug)]
pub(crate) struct EmbeddedMigrationCatalog {
    migrations: HashMap<String, (String, String)>,
    order: Vec<String>,
}

pub(crate) fn load_embedded_migration_catalog() -> Result<EmbeddedMigrationCatalog, String> {
    let embedded =
        scryer_infrastructure::list_embedded_migrations().map_err(|error| error.to_string())?;

    let mut migrations = HashMap::new();
    let mut order = Vec::with_capacity(embedded.len());

    for migration in embedded {
        order.push(migration.key.clone());
        migrations.insert(migration.key, (migration.checksum_algo, migration.checksum));
    }

    Ok(EmbeddedMigrationCatalog { migrations, order })
}

pub(crate) fn migration_key_preference_key(key: &str) -> (i64, &str) {
    let version = key
        .split('_')
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(-1);
    (version, key)
}

pub(crate) async fn admin_migrations_handler(database: Arc<SettingsStore>) -> Response {
    let applied = match database.list_applied_migrations().await {
        Ok(rows) => rows,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to load applied migrations: {error}"),
                }),
            )
                .into_response();
        }
    };

    let catalog = match load_embedded_migration_catalog() {
        Ok(rows) => rows,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to load embedded migrations: {error}"),
                }),
            )
                .into_response();
        }
    };

    let mut applied_lookup = HashMap::new();
    for status in &applied {
        applied_lookup.insert(status.migration_key.clone(), status.clone());
    }

    let mut migration_checksum_mismatch_flags = Vec::new();
    let mut applied_migrations = Vec::with_capacity(applied.len());
    let mut latest_successful_migration_key: Option<String> = None;

    for status in &applied {
        let expected = catalog.migrations.get(&status.migration_key).cloned();
        let expected_algo = expected.as_ref().map(|(algo, _)| algo.clone());
        let expected_checksum = expected.as_ref().map(|(_, checksum)| checksum.clone());
        let checksum_mismatch = expected.as_ref().is_none_or(|(algo, checksum)| {
            algo != &status.migration_checksum_algo || checksum != &status.migration_checksum
        });

        if checksum_mismatch {
            migration_checksum_mismatch_flags.push(status.migration_key.clone());
        }

        if status.success {
            let (version, key) = migration_key_preference_key(&status.migration_key);
            if latest_successful_migration_key
                .as_deref()
                .is_none_or(|current| {
                    let (current_version, current_key) = migration_key_preference_key(current);
                    (version, key) > (current_version, current_key)
                })
            {
                latest_successful_migration_key = Some(status.migration_key.clone());
            }
        }

        applied_migrations.push(AdminAppliedMigration {
            migration_key: status.migration_key.clone(),
            migration_checksum_algo: status.migration_checksum_algo.clone(),
            migration_checksum: status.migration_checksum.clone(),
            migration_checksum_algo_expected: expected_algo,
            migration_checksum_expected: expected_checksum,
            checksum_mismatch,
            applied_at: status.applied_at.clone(),
            success: status.success,
            error_message: status.error_message.clone(),
            runtime_version: status.runtime_version.clone(),
        });
    }

    let pending_migrations: Vec<String> = catalog
        .order
        .into_iter()
        .filter(|migration_key| !applied_lookup.contains_key(migration_key))
        .collect();

    Json(AdminMigrationsResponse {
        applied_migrations,
        pending_migrations,
        latest_successful_migration_key,
        migration_checksum_mismatch_flags,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_settings_scope_requires_catalog_settings_permission() {
        assert_eq!(
            permission_for_settings_scope(SETTINGS_SCOPE_MEDIA),
            AppPermission::ManageCatalogSettings
        );
    }

    #[test]
    fn system_settings_scope_requires_system_settings_permission() {
        assert_eq!(
            permission_for_settings_scope(SETTINGS_SCOPE_SYSTEM),
            AppPermission::ManageSystemSettings
        );
    }
}
