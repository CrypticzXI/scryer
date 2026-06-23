use super::*;

#[tokio::test]
async fn clear_title_image_cache_collapses_duplicate_requests_and_waits_for_scans() {
    let (app, admin) = bootstrap_with_user_repo(Arc::new(MockUserRepo::default()));
    let title_images = Arc::new(BlockingTitleImageRepo::default());
    let app = app.with_test_overrides(|services| services.with_title_images(title_images.clone()));

    let scan = app
        .runtime
        .library
        .library_scan_tracker
        .start_session(MediaFacet::Movie)
        .await
        .expect("scan should start");

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset")
    );
    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse queued reset")
    );

    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        0,
        "cache clear should wait behind active library scans"
    );

    app.runtime
        .library
        .library_scan_tracker
        .fail_session(&scan.session_id)
        .await
        .expect("scan should finish");
    wait_for_title_image_clear_calls(&title_images, 1).await;

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse running reset")
    );
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        1,
        "running cache clear should collapse duplicate requests"
    );

    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset after previous reset completes")
    );
    wait_for_title_image_clear_calls(&title_images, 2).await;
    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;
}
