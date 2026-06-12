impl AppUseCase {
    pub fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        if !actor
            .authorization
            .has_any_library_permission(scryer_domain::LibraryPermission::View)
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let (tx, rx) = broadcast::channel(32);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let event_types = vec![
                DomainEventType::DownloadQueueItemUpserted,
                DomainEventType::DownloadQueueItemRemoved,
            ];
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
            let mut cursor = match app
                .services
                .events
                .domain_events
                .list(&DomainEventFilter {
                    event_types: Some(event_types.clone()),
                    limit: 1,
                    ..DomainEventFilter::default()
                })
                .await
            {
                Ok(events) => events.first().map(|event| event.sequence).unwrap_or(0),
                Err(error) => {
                    tracing::warn!(
                        "download queue subscription initial cursor load failed: {error}"
                    );
                    return;
                }
            };

            let initial_items = match app.list_download_queue_snapshot(&actor).await {
                Ok(items) => items,
                Err(error) => {
                    tracing::warn!("download queue subscription initial load failed: {error}");
                    return;
                }
            };

            let mut items = initial_items
                .into_iter()
                .map(|item| (download_queue_projection_key(&item), item))
                .collect::<HashMap<_, _>>();

            loop {
                let batch = match app
                    .services
                    .events
                    .domain_events
                    .list(&DomainEventFilter {
                        event_types: Some(event_types.clone()),
                        after_sequence: Some(cursor),
                        limit: 100,
                        ..DomainEventFilter::default()
                    })
                    .await
                {
                    Ok(batch) => batch,
                    Err(error) => {
                        tracing::warn!("download queue subscription catch-up failed: {error}");
                        return;
                    }
                };
                if batch.is_empty() {
                    break;
                }

                let count = batch.len();
                for event in batch {
                    cursor = event.sequence;
                    apply_download_queue_projection_event(&mut items, &event);
                }
                if count < 100 {
                    break;
                }
            }

            let initial = match app
                .filter_download_queue_items_for_permission(
                    &actor,
                    sorted_download_queue_items(&items),
                    scryer_domain::LibraryPermission::View,
                )
                .await
            {
                Ok(items) => items,
                Err(error) => {
                    tracing::warn!("download queue subscription initial filter failed: {error}");
                    return;
                }
            };
            if tx.send(initial).is_err() {
                return;
            }

            loop {
                let next_events = match app
                    .services
                    .events
                    .domain_events
                    .list(&DomainEventFilter {
                        event_types: Some(event_types.clone()),
                        after_sequence: Some(cursor),
                        limit: 100,
                        ..DomainEventFilter::default()
                    })
                    .await
                {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => match wake_rx.recv().await {
                        Ok(sequence) => {
                            if sequence > cursor {
                                cursor = sequence.saturating_sub(1);
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(
                                "download queue subscription lagged, skipped {n} wakeups"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    Err(error) => {
                        tracing::warn!("download queue subscription replay failed: {error}");
                        break;
                    }
                };

                for event in next_events {
                    cursor = event.sequence;
                    if apply_download_queue_projection_event(&mut items, &event).is_some() {
                        let snapshot = match app
                            .filter_download_queue_items_for_permission(
                                &actor,
                                sorted_download_queue_items(&items),
                                scryer_domain::LibraryPermission::View,
                            )
                            .await
                        {
                            Ok(items) => items,
                            Err(error) => {
                                tracing::warn!(
                                    "download queue subscription event filter failed: {error}"
                                );
                                return;
                            }
                        };
                        if tx.send(snapshot).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}
