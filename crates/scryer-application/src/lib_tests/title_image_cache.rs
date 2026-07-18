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

struct ChunkedTitleImageRepo {
    pending: Mutex<Vec<TitleImageSyncTask>>,
}

impl ChunkedTitleImageRepo {
    fn new(count: usize) -> Self {
        Self {
            pending: Mutex::new(
                (0..count)
                    .map(|index| TitleImageSyncTask {
                        title_id: format!("title-{index}"),
                        kind: TitleImageKind::Poster,
                        source_url: format!("https://example.test/poster-{index}.jpg"),
                        variants: Vec::new(),
                    })
                    .collect(),
            ),
        }
    }

    async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }
}

#[async_trait]
impl TitleImageRepository for ChunkedTitleImageRepo {
    async fn list_title_image_refresh_work(
        &self,
        limit: usize,
        _skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(self
            .pending
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        self.pending.lock().await.clear();
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        self.pending
            .lock()
            .await
            .retain(|task| task.title_id != title_id);
        Ok(None)
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
    }
}

struct ChunkGatedTitleImageProcessor {
    started: AtomicUsize,
    first_chunk_gate: tokio::sync::Semaphore,
    second_chunk_gate: tokio::sync::Semaphore,
}

impl ChunkGatedTitleImageProcessor {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            first_chunk_gate: tokio::sync::Semaphore::new(0),
            second_chunk_gate: tokio::sync::Semaphore::new(0),
        }
    }
}

#[async_trait]
impl TitleImageProcessor for ChunkGatedTitleImageProcessor {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        _variants: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        const CHUNK_SIZE: usize = 8;

        let index = self.started.fetch_add(1, Ordering::SeqCst);
        let gate = if index < CHUNK_SIZE {
            &self.first_chunk_gate
        } else {
            &self.second_chunk_gate
        };
        gate.acquire()
            .await
            .expect("test image gate should remain open")
            .forget();

        Ok(TitleImageSourceResult {
            kind,
            requested_source_url: source_url.to_string(),
            source_url: source_url.to_string(),
            source_etag: None,
            source_last_modified: None,
            source_format: "jpeg".to_string(),
            source_width: 1,
            source_height: 1,
            variants: Vec::new(),
        })
    }
}

async fn wait_for_image_processor_starts(
    processor: &ChunkGatedTitleImageProcessor,
    expected: usize,
) {
    timeout(Duration::from_secs(5), async {
        while processor.started.load(Ordering::SeqCst) < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("image processing should reach expected count");
}

#[tokio::test]
async fn background_image_loop_yields_maintenance_lock_between_eight_item_chunks() {
    const CHUNK_SIZE: usize = 8;

    let (app, _) = bootstrap_with_user_repo(Arc::new(MockUserRepo::default()));
    let title_images = Arc::new(ChunkedTitleImageRepo::new(CHUNK_SIZE * 2));
    let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(title_images.clone())
            .with_title_image_processor(processor.clone())
    });
    let cancellation = tokio_util::sync::CancellationToken::new();
    let image_loop = tokio::spawn(
        crate::catalog::title_images::start_background_title_image_loop(
            app.clone(),
            cancellation.clone(),
        ),
    );
    app.runtime.catalog.poster_wake.notify_one();
    wait_for_image_processor_starts(&processor, 4).await;

    let writer_acquired = Arc::new(Notify::new());
    let release_writer = Arc::new(Notify::new());
    let maintenance_lock = app.runtime.catalog.title_image_maintenance_lock.clone();
    let writer_acquired_for_task = writer_acquired.clone();
    let release_writer_for_task = release_writer.clone();
    let writer = tokio::spawn(async move {
        let _guard = maintenance_lock.write().await;
        writer_acquired_for_task.notify_one();
        release_writer_for_task.notified().await;
    });
    sleep(Duration::from_millis(25)).await;

    processor.first_chunk_gate.add_permits(CHUNK_SIZE);
    timeout(Duration::from_secs(5), writer_acquired.notified())
        .await
        .expect("maintenance writer should acquire after the first image chunk");
    assert_eq!(
        processor.started.load(Ordering::SeqCst),
        CHUNK_SIZE,
        "the second image chunk must wait behind the maintenance writer"
    );

    release_writer.notify_one();
    writer.await.expect("maintenance writer should finish");
    wait_for_image_processor_starts(&processor, CHUNK_SIZE + 4).await;
    processor.second_chunk_gate.add_permits(CHUNK_SIZE);
    timeout(Duration::from_secs(5), async {
        while title_images.pending_count().await != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both image chunks should finish");

    cancellation.cancel();
    timeout(Duration::from_secs(5), image_loop)
        .await
        .expect("image loop should stop after cancellation")
        .expect("image loop task should not panic");
}
