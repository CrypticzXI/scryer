use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_graphql::{Context, Error, ErrorExtensions, Result as GqlResult};
use scryer_application::{
    AppError, AppUseCase, BackupRestorePreparedBundle, JwtSessionScope, LoginFailureTimingClass,
};
use scryer_domain::{AppPermission, LibraryPermission, User};
use tokio::sync::{broadcast, watch};

pub const LOGIN_FAILED_MESSAGE: &str = "Sign-in failed. Check your sign-in details and try again.";

/// Opaque handle to a log snapshot provider and subscription source.
/// The `scryer` crate constructs this from its `LogRingBuffer`.
#[derive(Clone)]
pub struct LogBuffer {
    snapshot_fn: Arc<dyn Fn(usize) -> Vec<String> + Send + Sync>,
    subscribe_fn: Arc<dyn Fn() -> broadcast::Receiver<String> + Send + Sync>,
}

impl LogBuffer {
    pub fn new(
        snapshot: impl Fn(usize) -> Vec<String> + Send + Sync + 'static,
        subscribe: impl Fn() -> broadcast::Receiver<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            snapshot_fn: Arc::new(snapshot),
            subscribe_fn: Arc::new(subscribe),
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        (self.snapshot_fn)(limit)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        (self.subscribe_fn)()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRuntimeStateSnapshot {
    pub form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
    pub effective_form_login_enabled: bool,
    pub webauthn_configured: bool,
    pub passkey_enabled: bool,
    pub env_override_active: bool,
    pub env_override_description: Option<String>,
    pub epoch: u64,
}

#[derive(Clone)]
pub struct AuthRuntimeStateHandle {
    snapshot: Arc<RwLock<AuthRuntimeStateSnapshot>>,
    epoch_tx: watch::Sender<u64>,
}

impl AuthRuntimeStateHandle {
    pub fn new(snapshot: AuthRuntimeStateSnapshot) -> Self {
        let (epoch_tx, _) = watch::channel(snapshot.epoch);
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
            epoch_tx,
        }
    }

    pub fn snapshot(&self) -> AuthRuntimeStateSnapshot {
        self.snapshot
            .read()
            .expect("auth runtime snapshot lock poisoned")
            .clone()
    }

    pub fn apply_saved_security_settings(
        &self,
        form_login_enabled: bool,
        skip_login_for_local_ips: bool,
    ) -> AuthRuntimeStateSnapshot {
        let next_snapshot = {
            let mut snapshot = self
                .snapshot
                .write()
                .expect("auth runtime snapshot lock poisoned");
            let previous_policy = (
                snapshot.effective_form_login_enabled,
                snapshot.effective_form_login_enabled && snapshot.skip_login_for_local_ips,
                snapshot.passkey_enabled,
            );
            snapshot.form_login_enabled = form_login_enabled;
            snapshot.skip_login_for_local_ips = skip_login_for_local_ips;
            if !snapshot.env_override_active {
                snapshot.effective_form_login_enabled = form_login_enabled;
            }
            snapshot.passkey_enabled =
                snapshot.webauthn_configured && snapshot.effective_form_login_enabled;
            let next_policy = (
                snapshot.effective_form_login_enabled,
                snapshot.effective_form_login_enabled && snapshot.skip_login_for_local_ips,
                snapshot.passkey_enabled,
            );
            if next_policy != previous_policy {
                snapshot.epoch += 1;
            }
            snapshot.clone()
        };

        let _ = self.epoch_tx.send(next_snapshot.epoch);
        next_snapshot
    }

    pub fn subscribe_epoch(&self) -> watch::Receiver<u64> {
        self.epoch_tx.subscribe()
    }
}

#[derive(Clone, Copy)]
pub struct ConnectionAuthEpoch(pub u64);

#[derive(Clone, Copy, Default)]
pub struct MfaVerification {
    pub verified_until: Option<i64>,
    pub step_up_verified_until: Option<i64>,
    pub session_scope: JwtSessionScope,
}

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub auth_runtime: AuthRuntimeStateHandle,
    pub restore: Option<RestoreContext>,
}

#[derive(Clone)]
pub struct RestoreRestartHandle {
    schedule_fn: Arc<dyn Fn() + Send + Sync>,
}

pub struct RestoreSqliteDatastoreRequest {
    pub target_db_path: PathBuf,
    pub migration_mode: RestoreMigrationMode,
    pub bundle_path: PathBuf,
    pub passphrase: Option<String>,
}

#[derive(Clone)]
pub struct RestoreDatastoreHandle {
    restore_sqlite_fn: Arc<
        dyn Fn(RestoreSqliteDatastoreRequest) -> Result<BackupRestorePreparedBundle, AppError>
            + Send
            + Sync,
    >,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreDatastoreEngine {
    Sqlite,
    Postgres,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreMigrationMode {
    ValidateOnly,
    Apply,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreDatastoreConfig {
    pub engine: RestoreDatastoreEngine,
    pub migration_mode: RestoreMigrationMode,
}

impl RestoreRestartHandle {
    pub fn new(schedule: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            schedule_fn: Arc::new(schedule),
        }
    }

    pub fn schedule_restart(&self) {
        (self.schedule_fn)();
    }
}

impl RestoreDatastoreHandle {
    pub fn new(
        restore_sqlite: impl Fn(
            RestoreSqliteDatastoreRequest,
        ) -> Result<BackupRestorePreparedBundle, AppError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            restore_sqlite_fn: Arc::new(restore_sqlite),
        }
    }

    pub fn unavailable() -> Self {
        Self::new(|_| {
            Err(AppError::Validation(
                "restore datastore operations are not configured".into(),
            ))
        })
    }

    pub fn restore_sqlite_bundle_to_path(
        &self,
        request: RestoreSqliteDatastoreRequest,
    ) -> Result<BackupRestorePreparedBundle, AppError> {
        (self.restore_sqlite_fn)(request)
    }
}

#[derive(Clone)]
pub struct RestoreContext {
    pub data_dir: PathBuf,
    pub datastore_config: RestoreDatastoreConfig,
    pub datastore: RestoreDatastoreHandle,
    pub restart: RestoreRestartHandle,
}

pub fn app_from_ctx(ctx: &Context<'_>) -> GqlResult<AppUseCase> {
    Ok(ctx.data_unchecked::<ApiContext>().app.clone())
}

pub fn auth_runtime_from_ctx(ctx: &Context<'_>) -> AuthRuntimeStateHandle {
    ctx.data_unchecked::<ApiContext>().auth_runtime.clone()
}

pub fn restore_context_from_ctx(ctx: &Context<'_>) -> GqlResult<RestoreContext> {
    ctx.data_unchecked::<ApiContext>()
        .restore
        .clone()
        .ok_or_else(|| Error::new("restore is not configured"))
}

pub fn to_gql_error(err: AppError) -> Error {
    match err {
        AppError::DownloadFeedbackTimeout(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "DOWNLOAD_FEEDBACK_TIMEOUT");
            })
        }
        AppError::PluginInstallInProgress(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "PLUGIN_INSTALL_IN_PROGRESS");
            })
        }
        AppError::TotpStepUpRequired(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "TOTP_STEP_UP_REQUIRED");
            })
        }
        AppError::TotpEnrollmentRequired(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "TOTP_ENROLLMENT_REQUIRED");
            })
        }
        AppError::MfaEnrollmentRequired(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "MFA_ENROLLMENT_REQUIRED");
            })
        }
        AppError::TotpInvalidCode(message) => Error::new(message).extend_with(|_, extensions| {
            extensions.set("code", "TOTP_INVALID_CODE");
        }),
        AppError::TotpRecoveryCodeUsed(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "TOTP_RECOVERY_CODE_USED");
            })
        }
        _ => Error::new(err.to_string()),
    }
}

fn login_progression_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::TotpStepUpRequired(_)
            | AppError::TotpEnrollmentRequired(_)
            | AppError::MfaEnrollmentRequired(_)
            | AppError::TotpInvalidCode(_)
            | AppError::TotpRecoveryCodeUsed(_)
    )
}

fn app_error_kind(err: &AppError) -> &'static str {
    match err {
        AppError::Unauthorized(_) => "Unauthorized",
        AppError::Validation(_) => "Validation",
        AppError::PluginInstallInProgress(_) => "PluginInstallInProgress",
        AppError::NotFound(_) => "NotFound",
        AppError::DownloadFeedbackTimeout(_) => "DownloadFeedbackTimeout",
        AppError::DownloadSubmitAmbiguous(_) => "DownloadSubmitAmbiguous",
        AppError::TotpStepUpRequired(_) => "TotpStepUpRequired",
        AppError::TotpEnrollmentRequired(_) => "TotpEnrollmentRequired",
        AppError::MfaEnrollmentRequired(_) => "MfaEnrollmentRequired",
        AppError::TotpInvalidCode(_) => "TotpInvalidCode",
        AppError::TotpRecoveryCodeUsed(_) => "TotpRecoveryCodeUsed",
        AppError::Repository(_) => "Repository",
    }
}

pub fn to_login_gql_error(method: &'static str, err: AppError) -> Error {
    if login_progression_error(&err) {
        return to_gql_error(err);
    }

    let error_kind = app_error_kind(&err);
    if matches!(err, AppError::Repository(_)) {
        tracing::warn!(login_method = method, error_kind, "masked login failure");
    } else {
        tracing::debug!(login_method = method, error_kind, "masked login failure");
    }
    Error::new(LOGIN_FAILED_MESSAGE).extend_with(|_, extensions| {
        extensions.set("code", "LOGIN_FAILED");
    })
}

pub async fn to_login_gql_error_after_timing(
    method: &'static str,
    timing_class: LoginFailureTimingClass,
    started_at: Instant,
    err: AppError,
) -> Error {
    if login_progression_error(&err) {
        return to_gql_error(err);
    }

    AppUseCase::apply_login_failure_timing(timing_class, started_at).await;
    to_login_gql_error(method, err)
}

pub fn actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if mfa_verification_from_ctx(ctx).session_scope == JwtSessionScope::MfaEnrollment {
        return Err(to_gql_error(AppError::MfaEnrollmentRequired(
            "MFA enrollment must be completed before accessing Scryer".into(),
        )));
    }
    current_user_any_scope_from_ctx(ctx).ok_or_else(|| Error::new("authentication required"))
}

pub fn mfa_enrollment_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if mfa_verification_from_ctx(ctx).session_scope != JwtSessionScope::MfaEnrollment {
        return Err(to_gql_error(AppError::MfaEnrollmentRequired(
            "MFA enrollment session required".into(),
        )));
    }
    current_user_any_scope_from_ctx(ctx).ok_or_else(|| Error::new("authentication required"))
}

pub async fn require_app_permission(
    ctx: &Context<'_>,
    permission: AppPermission,
) -> GqlResult<User> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.require_app_permission(&actor, permission)
        .await
        .map_err(to_gql_error)?;
    Ok(actor)
}

pub async fn actor_has_app_permission(
    ctx: &Context<'_>,
    permission: AppPermission,
) -> GqlResult<bool> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.has_app_permission(&actor, permission)
        .await
        .map_err(to_gql_error)
}

pub async fn actor_has_any_library_permission(
    ctx: &Context<'_>,
    permission: LibraryPermission,
) -> GqlResult<bool> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.has_any_library_permission(&actor, permission)
        .await
        .map_err(to_gql_error)
}

pub fn current_user_from_ctx(ctx: &Context<'_>) -> Option<User> {
    if mfa_verification_from_ctx(ctx).session_scope == JwtSessionScope::MfaEnrollment {
        return None;
    }
    current_user_any_scope_from_ctx(ctx)
}

fn current_user_any_scope_from_ctx(ctx: &Context<'_>) -> Option<User> {
    if let Some(connection_epoch) = ctx.data_opt::<ConnectionAuthEpoch>()
        && connection_epoch.0 != auth_runtime_from_ctx(ctx).snapshot().epoch
    {
        return None;
    }

    ctx.data_opt::<User>().cloned()
}

pub fn mfa_verification_from_ctx(ctx: &Context<'_>) -> MfaVerification {
    ctx.data_opt::<MfaVerification>()
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphql_error_code(error: &Error) -> Option<&str> {
        error
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code"))
            .and_then(|value| match value {
                async_graphql::Value::String(value) => Some(value.as_str()),
                _ => None,
            })
    }

    #[test]
    fn login_errors_mask_disclosure_details() {
        for err in [
            AppError::Unauthorized("external account is not invited".into()),
            AppError::NotFound("user 00000000-0000-0000-0000-000000000001".into()),
            AppError::Validation("passkeys require a password-backed account".into()),
        ] {
            let error = to_login_gql_error("jellyfin", err);
            assert_eq!(error.message, LOGIN_FAILED_MESSAGE);
            assert_eq!(graphql_error_code(&error), Some("LOGIN_FAILED"));
        }
    }

    #[test]
    fn login_errors_preserve_mfa_progression() {
        let error = to_login_gql_error(
            "local",
            AppError::TotpStepUpRequired("TOTP code is required for local login".into()),
        );
        assert_eq!(error.message, "TOTP code is required for local login");
        assert_eq!(graphql_error_code(&error), Some("TOTP_STEP_UP_REQUIRED"));
    }
}
