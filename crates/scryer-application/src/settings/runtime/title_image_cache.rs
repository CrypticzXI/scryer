struct TitleImageCacheClearScheduledGuard {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for TitleImageCacheClearScheduledGuard {
    fn drop(&mut self) {
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}
impl AppUseCase {
    pub async fn clear_title_image_cache(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let scheduled = self
            .runtime
            .catalog
            .title_image_cache_clear_scheduled
            .clone();
        if scheduled.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Ok(true);
        }

        let app = self.clone();
        tokio::spawn(async move {
            let _scheduled_guard = TitleImageCacheClearScheduledGuard { flag: scheduled };
            let _maintenance_guard = loop {
                let active_scans = app.runtime.library.library_scan_tracker.list_active().await;
                if !active_scans.is_empty() {
                    info!(
                        active_scans = active_scans.len(),
                        "title image cache reset pausing while library scan is active"
                    );
                    app.runtime
                        .library
                        .library_scan_tracker
                        .wait_until_idle()
                        .await;
                    info!("title image cache reset resuming after library scan");
                }
                let guard = app
                    .runtime
                    .catalog
                    .title_image_maintenance_lock
                    .write()
                    .await;
                if app
                    .runtime
                    .library
                    .library_scan_tracker
                    .list_active()
                    .await
                    .is_empty()
                {
                    break guard;
                }
            };
            match app
                .services
                .library
                .title_images
                .clear_title_image_cache()
                .await
            {
                Ok(()) => {
                    info!("title image cache reset completed");
                }
                Err(error) => {
                    warn!(error = %error, "title image cache reset failed");
                }
            }
            app.wake_title_image_loops();
        });

        Ok(true)
    }
}
