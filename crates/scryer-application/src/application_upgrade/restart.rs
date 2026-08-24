use std::sync::Arc;

/// Process-restart callback supplied by the executable host.
///
/// The application crate owns this small boundary so an upgrade can schedule
/// its restart without depending on an HTTP or GraphQL layer.
#[derive(Clone)]
pub struct ApplicationUpgradeRestartHandle {
    schedule_fn: Arc<dyn Fn() + Send + Sync>,
}

impl ApplicationUpgradeRestartHandle {
    pub fn new(schedule: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            schedule_fn: Arc::new(schedule),
        }
    }

    pub fn schedule_restart(&self) {
        (self.schedule_fn)();
    }
}
