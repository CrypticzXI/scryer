use std::sync::{Arc, RwLock};

use async_graphql::{Context, Error, ErrorExtensions, Result as GqlResult, Schema};
use scryer_application::AppError;
use scryer_application::AppUseCase;
use scryer_domain::User;
use tokio::sync::{broadcast, watch};

use crate::{mutation::MutationRoot, query::QueryRoot, subscription::SubscriptionRoot};

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

pub type ApiSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRuntimeStateSnapshot {
    pub form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
    pub effective_form_login_enabled: bool,
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
            );
            snapshot.form_login_enabled = form_login_enabled;
            snapshot.skip_login_for_local_ips = skip_login_for_local_ips;
            if !snapshot.env_override_active {
                snapshot.effective_form_login_enabled = form_login_enabled;
            }
            let next_policy = (
                snapshot.effective_form_login_enabled,
                snapshot.effective_form_login_enabled && snapshot.skip_login_for_local_ips,
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

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub auth_runtime: AuthRuntimeStateHandle,
}

pub fn build_schema(app: AppUseCase, auth_runtime: AuthRuntimeStateHandle) -> ApiSchema {
    build_schema_with_log_buffer(app, auth_runtime, None)
}

pub fn build_schema_with_log_buffer(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
) -> ApiSchema {
    let mut builder = Schema::build(QueryRoot, MutationRoot::default(), SubscriptionRoot)
        .data(ApiContext { app, auth_runtime });
    if let Some(buf) = log_buffer {
        builder = builder.data(buf);
    }
    builder.finish()
}

pub(crate) fn app_from_ctx(ctx: &Context<'_>) -> GqlResult<AppUseCase> {
    Ok(ctx.data_unchecked::<ApiContext>().app.clone())
}

pub(crate) fn auth_runtime_from_ctx(ctx: &Context<'_>) -> AuthRuntimeStateHandle {
    ctx.data_unchecked::<ApiContext>().auth_runtime.clone()
}

pub(crate) fn to_gql_error(err: AppError) -> Error {
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
        _ => Error::new(err.to_string()),
    }
}

pub(crate) fn actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    current_user_from_ctx(ctx).ok_or_else(|| Error::new("authentication required"))
}

pub(crate) fn current_user_from_ctx(ctx: &Context<'_>) -> Option<User> {
    if let Some(connection_epoch) = ctx.data_opt::<ConnectionAuthEpoch>()
        && connection_epoch.0 != auth_runtime_from_ctx(ctx).snapshot().epoch
    {
        return None;
    }

    ctx.data_opt::<User>().cloned()
}
