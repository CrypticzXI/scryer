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

pub(crate) async fn ensure_admin_password_configured(
    app_use_case: &AppUseCase,
) -> Result<(), String> {
    if app_use_case
        .existing_default_admin_uses_bootstrap_password()
        .await
        .map_err(|error| format!("failed to validate default admin password state: {error}"))?
    {
        return Err(
            "form login is enabled, but the default admin password is still 'admin'; change it before enabling auth".to_string(),
        );
    }

    if !app_use_case
        .usable_admin_login_exists()
        .await
        .map_err(|error| format!("failed to validate admin login state: {error}"))?
    {
        return Err(
            "form login is enabled, but no local full-admin user has a usable password; start with SCRYER_RECOVERY_ADMIN_PASSWORD set to recover the instance".to_string(),
        );
    }

    Ok(())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_id: Option<String>,
}

impl ErrorResponse {
    pub(crate) fn new(error: String) -> Self {
        Self {
            error,
            error_id: None,
        }
    }

    pub(crate) fn with_error_id(error: String, error_id: String) -> Self {
        Self {
            error,
            error_id: Some(error_id),
        }
    }
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

pub(crate) async fn admin_migrations_handler(
    database: Arc<SettingsStore>,
    app_use_case: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    headers: HeaderMap,
    remote_addr: SocketAddr,
) -> Response {
    let _actor = match resolve_actor_with_app_permission(
        &app_use_case,
        &auth_runtime,
        &headers,
        Some(remote_addr),
        AppPermission::ManageSystemSettings,
    )
    .await
    {
        Ok(actor) => actor,
        Err(error) => return map_app_error(error),
    };

    let applied = match database.list_applied_migrations().await {
        Ok(rows) => rows,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(format!(
                    "failed to load applied migrations: {error}"
                ))),
            )
                .into_response();
        }
    };

    let catalog = match load_embedded_migration_catalog() {
        Ok(rows) => rows,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(format!(
                    "failed to load embedded migrations: {error}"
                ))),
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
    use std::net::Ipv4Addr;

    use async_trait::async_trait;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::ConnectInfo;
    use axum::http::{Request, header};
    use axum::routing::get;
    use scryer_application::{
        AppError, AppResult, AppServices, DownloadClient, DownloadClientAddRequest,
        DownloadGrabResult, FacetRegistry, IndexerClient, IndexerRoutingPlan,
        IndexerSearchResponse, JwtAuthConfig, SearchMode,
    };
    use scryer_application::{
        NullAcquisitionStateRepository, NullBlocklistRepository, NullDomainEventRepository,
        NullDownloadSubmissionRepository, NullHousekeepingRepository, NullImportArtifactRepository,
        NullImportRepository, NullJobRunRepository, NullLibraryProbeRepository,
        NullLibraryScanUnmatchedItemRepository, NullMediaFileRepository,
        NullPendingReleaseRepository, NullPluginInstallationRepository,
        NullPostProcessingScriptRepository, NullRuleSetRepository, NullSubtitleDownloadRepository,
        NullTitleImageRepository, NullWantedItemRepository, NullWorkflowOperationRepository,
    };
    use scryer_infrastructure::sqlite::{
        LibraryStore, QualityProfileStore, ShowStore, TitleStore, UserStore,
    };
    use scryer_infrastructure::{
        DownloadClientConfigStore, EncryptionKey, FileSystemLibraryScanner, IndexerConfigStore,
        MetadataGatewayClient, ReleaseStore, SmgEnrollmentConfig, SqliteServices,
    };
    use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::middleware::{
        AuthlessAccessGuardState, AuthlessAccessPolicy, enforce_authless_access_guard,
    };

    #[derive(Default)]
    struct NullIndexerClient;

    #[async_trait]
    impl IndexerClient for NullIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        ) -> AppResult<IndexerSearchResponse> {
            Ok(IndexerSearchResponse {
                results: Vec::new(),
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    #[derive(Default)]
    struct NullDownloadClient;

    #[async_trait]
    impl DownloadClient for NullDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not configured".into()))
        }
    }

    struct AdminMigrationsFixture {
        app: AppUseCase,
        auth_runtime: AuthRuntimeStateHandle,
        settings_store: Arc<SettingsStore>,
        _db: SqliteServices,
        _backup_dir: tempfile::TempDir,
    }

    async fn admin_migrations_fixture(auth_enabled: bool) -> AdminMigrationsFixture {
        scryer_infrastructure::keystore::disable_platform_keystore_for_tests();

        let db = SqliteServices::new(":memory:")
            .await
            .expect("failed to create in-memory SQLite");
        db.set_encryption_key(EncryptionKey::generate())
            .await
            .expect("failed to configure test encryption key");
        let datastore = db.datastore();
        let backup_dir = tempfile::TempDir::new().expect("failed to create backup tempdir");
        let settings_store = Arc::new(SettingsStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));

        let services = AppServices::builder(
            Arc::new(TitleStore::new(datastore.clone())),
            Arc::new(ShowStore::new(datastore.clone())),
            Arc::new(UserStore::new(datastore.clone())),
            Arc::new(IndexerConfigStore::new(
                datastore.clone(),
                db.encryption_key_state(),
            )),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(DownloadClientConfigStore::new(
                datastore.clone(),
                db.encryption_key_state(),
            )),
            Arc::new(ReleaseStore::new(
                datastore.clone(),
                db.encryption_key_state(),
            )),
            settings_store.clone(),
            Arc::new(QualityProfileStore::new(datastore.clone())),
            backup_dir.path().to_path_buf(),
        )
        .with_libraries(Arc::new(LibraryStore::new(datastore.clone())))
        .with_metadata_gateway(Arc::new(MetadataGatewayClient::new(
            "http://127.0.0.1:9/graphql".to_string(),
            db.clone(),
            SmgEnrollmentConfig {
                registration_secret: None,
            },
        )))
        .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
        .with_domain_events(Arc::new(NullDomainEventRepository))
        .with_imports(Arc::new(NullImportRepository))
        .with_workflow_operations(Arc::new(NullWorkflowOperationRepository))
        .with_import_artifacts(Arc::new(NullImportArtifactRepository))
        .with_media_files(Arc::new(NullMediaFileRepository))
        .with_acquisition_state(Arc::new(NullAcquisitionStateRepository))
        .with_download_submissions(Arc::new(NullDownloadSubmissionRepository))
        .with_wanted_items(Arc::new(NullWantedItemRepository))
        .with_rule_sets(Arc::new(NullRuleSetRepository))
        .with_pp_scripts(Arc::new(NullPostProcessingScriptRepository))
        .with_plugin_installations(Arc::new(NullPluginInstallationRepository))
        .with_system_info(settings_store.clone())
        .with_title_images(Arc::new(NullTitleImageRepository))
        .with_housekeeping(Arc::new(NullHousekeepingRepository))
        .with_pending_releases(Arc::new(NullPendingReleaseRepository))
        .with_blocklist_repo(Arc::new(NullBlocklistRepository))
        .with_subtitle_downloads(Arc::new(NullSubtitleDownloadRepository))
        .with_job_runs(Arc::new(NullJobRunRepository))
        .with_library_probe_signatures(Arc::new(NullLibraryProbeRepository))
        .with_library_scan_unmatched_items(Arc::new(NullLibraryScanUnmatchedItemRepository))
        .build();

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-admin-migrations-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        );
        let auth_runtime = AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
            form_login_enabled: auth_enabled,
            skip_login_for_local_ips: false,
            effective_form_login_enabled: auth_enabled,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        });

        AdminMigrationsFixture {
            app,
            auth_runtime,
            settings_store,
            _db: db,
            _backup_dir: backup_dir,
        }
    }

    fn protected_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: false,
            recovery_mode: false,
        }
    }

    fn admin_migrations_test_app(fixture: &AdminMigrationsFixture) -> Router {
        let settings_store = fixture.settings_store.clone();
        let app = fixture.app.clone();
        let auth_runtime = fixture.auth_runtime.clone();
        let guard_state = AuthlessAccessGuardState {
            auth_runtime: auth_runtime.clone(),
            policy: protected_authless_policy(),
        };

        Router::new()
            .route(
                "/admin/migrations",
                get(
                    move |headers: HeaderMap, ConnectInfo(remote_addr): ConnectInfo<SocketAddr>| {
                        admin_migrations_handler(
                            settings_store.clone(),
                            app.clone(),
                            auth_runtime.clone(),
                            headers,
                            remote_addr,
                        )
                    },
                ),
            )
            .layer(axum::middleware::from_fn_with_state(
                guard_state,
                enforce_authless_access_guard,
            ))
    }

    fn request_with_peer(
        uri: &str,
        peer: SocketAddr,
        authorization: Option<&str>,
    ) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(authorization) = authorization {
            builder = builder.header(header::AUTHORIZATION, authorization);
        }
        let mut request = builder.body(Body::empty()).expect("request");
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    async fn response_json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("json response")
    }

    #[tokio::test]
    async fn admin_migrations_requires_auth_when_auth_enabled() {
        let fixture = admin_migrations_fixture(true).await;
        let app = admin_migrations_test_app(&fixture);

        let response = app
            .oneshot(request_with_peer(
                "/admin/migrations",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                None,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_migrations_rejects_invalid_auth_when_auth_enabled() {
        let fixture = admin_migrations_fixture(true).await;
        let app = admin_migrations_test_app(&fixture);

        let response = app
            .oneshot(request_with_peer(
                "/admin/migrations",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                Some("Bearer not-a-real-token"),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_migrations_allows_authorized_admin() {
        let fixture = admin_migrations_fixture(true).await;
        let admin = fixture
            .app
            .find_or_create_default_user()
            .await
            .expect("default admin");
        let token = fixture
            .app
            .issue_access_token(&admin)
            .await
            .expect("access token");
        let app = admin_migrations_test_app(&fixture);

        let response = app
            .oneshot(request_with_peer(
                "/admin/migrations",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                Some(&format!("Bearer {token}")),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body["applied_migrations"].is_array());
        assert!(body["pending_migrations"].is_array());
        assert!(body["migration_checksum_mismatch_flags"].is_array());
    }

    #[tokio::test]
    async fn admin_migrations_allows_auth_disabled_private_client() {
        let fixture = admin_migrations_fixture(false).await;
        fixture
            .app
            .find_or_create_default_user()
            .await
            .expect("default admin");
        let app = admin_migrations_test_app(&fixture);

        let response = app
            .oneshot(request_with_peer(
                "/admin/migrations",
                SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                None,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body["applied_migrations"].is_array());
        assert!(body["pending_migrations"].is_array());
    }

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
