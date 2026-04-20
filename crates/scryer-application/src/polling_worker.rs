use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub(crate) struct PollingWorker {
    name: &'static str,
    token: CancellationToken,
}

impl PollingWorker {
    pub(crate) fn new(name: &'static str, token: CancellationToken) -> Self {
        info!(worker = name, "background worker started");
        Self { name, token }
    }

    pub(crate) async fn wait_for_tick(&self, interval: &mut tokio::time::Interval) -> bool {
        tokio::select! {
            _ = self.token.cancelled() => {
                self.log_shutdown();
                false
            }
            _ = interval.tick() => true,
        }
    }

    pub(crate) async fn wait_for_wake_or_timeout(&self, wake: &Notify, timeout: Duration) -> bool {
        tokio::select! {
            _ = self.token.cancelled() => {
                self.log_shutdown();
                false
            }
            _ = wake.notified() => true,
            _ = tokio::time::sleep(timeout) => true,
        }
    }

    pub(crate) async fn wait_for_future_or_wake_or_timeout<F>(
        &self,
        wake: &Notify,
        future: F,
        timeout: Duration,
    ) -> bool
    where
        F: Future<Output = ()>,
    {
        tokio::select! {
            _ = self.token.cancelled() => {
                self.log_shutdown();
                false
            }
            _ = wake.notified() => true,
            _ = future => true,
            _ = tokio::time::sleep(timeout) => true,
        }
    }

    pub(crate) async fn wait_for_sleep(&self, duration: Duration) -> bool {
        tokio::select! {
            _ = self.token.cancelled() => {
                self.log_shutdown();
                false
            }
            _ = tokio::time::sleep(duration) => true,
        }
    }

    pub(crate) fn warn_error(&self, context: &'static str, error: &impl Display) {
        metrics::counter!("scryer_background_worker_errors_total", "worker" => self.name, "context" => context)
            .increment(1);
        warn!(worker = self.name, context, error = %error, "background worker error");
    }

    pub(crate) fn warn_recovered(&self, context: &'static str, recovered: u64) {
        metrics::counter!("scryer_background_worker_stale_recoveries_total", "worker" => self.name, "context" => context)
            .increment(recovered);
        warn!(
            worker = self.name,
            context, recovered, "background worker recovered stale work"
        );
    }

    fn log_shutdown(&self) {
        info!(worker = self.name, "background worker shutting down");
    }
}
