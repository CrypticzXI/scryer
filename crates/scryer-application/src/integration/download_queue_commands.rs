use crate::AppUseCase;
use crate::polling_worker::PollingWorker;

const DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS: u64 = 2;
const DOWNLOAD_DELETE_STALE_RECOVERY_SECONDS: i64 = 120;

pub async fn start_background_download_delete_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("download_delete_poller", token);
    tracing::info!(
        interval_seconds = DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS,
        "download delete poller started"
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS,
    ));

    loop {
        if !worker.wait_for_tick(&mut interval).await {
            return;
        }

        match app
            .services
            .workflow
            .download_queue_commands
            .recover_stale_running_delete_commands(DOWNLOAD_DELETE_STALE_RECOVERY_SECONDS)
            .await
        {
            Ok(recovered) if recovered > 0 => {
                worker.warn_recovered("recover_stale_running_delete_commands", recovered);
            }
            Err(error) => {
                worker.warn_error("recover_stale_running_delete_commands", &error);
            }
            _ => {}
        }

        let pending = match app
            .services
            .workflow
            .download_queue_commands
            .list_pending_delete_commands()
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                worker.warn_error("list_pending_delete_commands", &error);
                continue;
            }
        };

        for command in pending {
            if let Err(error) = app
                .services
                .workflow
                .download_queue_commands
                .mark_delete_command_running(&command.id)
                .await
            {
                worker.warn_error("mark_delete_command_running", &error);
                continue;
            }

            let result = app
                .services
                .integrations
                .download_client
                .delete_queue_item_for_client(
                    &command.client_type,
                    &command.download_client_item_id,
                    command.is_history,
                )
                .await;

            match result {
                Ok(()) => {
                    if let Err(error) = app
                        .services
                        .workflow
                        .download_queue_commands
                        .mark_delete_command_completed(&command.id)
                        .await
                    {
                        worker.warn_error("mark_delete_command_completed", &error);
                    }
                }
                Err(error) => {
                    let error_text = error.to_string();
                    if let Err(update_error) = app
                        .services
                        .workflow
                        .download_queue_commands
                        .mark_delete_command_failed(&command.id, Some(&error_text))
                        .await
                    {
                        worker.warn_error("mark_delete_command_failed", &update_error);
                    }
                }
            }
        }
    }
}
