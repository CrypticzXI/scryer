use std::time::Duration;

use scryer_application::{AppError, AppResult};

const PLUGIN_BLOCKING_GRACE: Duration = Duration::from_secs(2);

pub(crate) async fn run_blocking_plugin_call<F, T>(
    timeout: Duration,
    label: &'static str,
    call: F,
) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let deadline = timeout
        .checked_add(PLUGIN_BLOCKING_GRACE)
        .unwrap_or(timeout);
    let task = tokio::task::spawn_blocking(call);
    match tokio::time::timeout(deadline, task).await {
        Ok(joined) => joined
            .map_err(|error| AppError::Repository(format!("{label} task panicked: {error}")))?,
        Err(_) => Err(AppError::Repository(format!(
            "{label} timed out after {} seconds",
            timeout.as_secs()
        ))),
    }
}
