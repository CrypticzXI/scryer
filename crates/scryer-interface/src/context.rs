use std::sync::Arc;

use async_graphql::{Context, Error, ErrorExtensions, Result as GqlResult, Schema};
use scryer_application::AppError;
use scryer_application::AppUseCase;
use scryer_domain::User;
use tokio::sync::broadcast;

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

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub auth_enabled: bool,
}

pub fn build_schema(app: AppUseCase, auth_enabled: bool) -> ApiSchema {
    build_schema_with_log_buffer(app, auth_enabled, None)
}

pub fn build_schema_with_log_buffer(
    app: AppUseCase,
    auth_enabled: bool,
    log_buffer: Option<LogBuffer>,
) -> ApiSchema {
    let mut builder = Schema::build(QueryRoot, MutationRoot::default(), SubscriptionRoot)
        .data(ApiContext { app, auth_enabled });
    if let Some(buf) = log_buffer {
        builder = builder.data(buf);
    }
    builder.finish()
}

pub(crate) fn app_from_ctx(ctx: &Context<'_>) -> GqlResult<AppUseCase> {
    Ok(ctx.data_unchecked::<ApiContext>().app.clone())
}

pub(crate) fn to_gql_error(err: AppError) -> Error {
    match err {
        AppError::DownloadFeedbackTimeout(message) => {
            Error::new(message).extend_with(|_, extensions| {
                extensions.set("code", "DOWNLOAD_FEEDBACK_TIMEOUT");
            })
        }
        _ => Error::new(err.to_string()),
    }
}

pub(crate) fn actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    ctx.data_opt::<User>()
        .cloned()
        .ok_or_else(|| Error::new("authentication required"))
}
