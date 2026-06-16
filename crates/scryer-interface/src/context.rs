use async_graphql::Schema;
use scryer_application::AppUseCase;

use crate::{mutation::MutationRoot, query::QueryRoot, subscription::SubscriptionRoot};

pub use scryer_interface_core::{
    ApiContext, AuthRuntimeStateHandle, AuthRuntimeStateSnapshot, ConnectionAuthEpoch, LogBuffer,
    MfaVerification, RestoreContext, RestoreDatastoreConfig, RestoreDatastoreEngine,
    RestoreDatastoreHandle, RestoreMigrationMode, RestoreRestartHandle,
    RestoreSqliteDatastoreRequest, actor_from_ctx, actor_has_any_library_permission,
    actor_has_app_permission, app_from_ctx, auth_runtime_from_ctx, current_user_from_ctx,
    mfa_verification_from_ctx, require_app_permission, require_config_step_up,
    restore_context_from_ctx, to_gql_error, to_login_gql_error_after_timing,
};

pub type ApiSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

pub fn build_schema(app: AppUseCase, auth_runtime: AuthRuntimeStateHandle) -> ApiSchema {
    build_schema_with_log_buffer_and_restore(app, auth_runtime, None, None)
}

pub fn build_schema_with_log_buffer(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
) -> ApiSchema {
    build_schema_with_log_buffer_and_restore(app, auth_runtime, log_buffer, None)
}

pub fn build_schema_with_log_buffer_and_restore(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
    restore: Option<RestoreContext>,
) -> ApiSchema {
    let mut builder = Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        SubscriptionRoot,
    )
    .data(ApiContext {
        app,
        auth_runtime,
        restore,
    });
    if let Some(buf) = log_buffer {
        builder = builder.data(buf);
    }
    builder.finish()
}
