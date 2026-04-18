use crate::{AppServicesBuilder, AppUseCase};

pub trait AppUseCaseTestExt {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder;

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64>;
}

impl AppUseCaseTestExt for AppUseCase {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        AppUseCase::with_test_overrides(self, configure)
    }

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64> {
        self.runtime.notification_event_broadcast.subscribe()
    }
}
