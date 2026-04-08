use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct PostHydrationTitleScanQueueState {
    queued: VecDeque<String>,
    queued_set: HashSet<String>,
    running: HashSet<String>,
    rerun_requested: HashSet<String>,
}

#[derive(Clone, Default)]
pub struct PostHydrationTitleScanQueue {
    state: Arc<Mutex<PostHydrationTitleScanQueueState>>,
    wake: Arc<Notify>,
}

impl PostHydrationTitleScanQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(&self, title_id: String) -> bool {
        let should_wake = {
            let mut state = self.state.lock().await;
            if state.queued_set.contains(&title_id) {
                return false;
            }

            if state.running.contains(&title_id) {
                state.rerun_requested.insert(title_id);
                return false;
            }

            state.queued_set.insert(title_id.clone());
            state.queued.push_back(title_id);
            true
        };

        if should_wake {
            self.wake.notify_one();
        }

        true
    }

    pub async fn dequeue(&self, token: &CancellationToken) -> Option<String> {
        loop {
            if let Some(title_id) = {
                let mut state = self.state.lock().await;
                let next = state.queued.pop_front();
                if let Some(title_id) = next.as_ref() {
                    state.queued_set.remove(title_id);
                    state.running.insert(title_id.clone());
                }
                next
            } {
                return Some(title_id);
            }

            tokio::select! {
                _ = token.cancelled() => return None,
                _ = self.wake.notified() => {}
            }
        }
    }

    pub async fn finish(&self, title_id: &str) {
        let should_wake = {
            let mut state = self.state.lock().await;
            state.running.remove(title_id);

            if state.rerun_requested.remove(title_id) {
                let title_id = title_id.to_string();
                state.queued_set.insert(title_id.clone());
                state.queued.push_back(title_id);
                true
            } else {
                false
            }
        };

        if should_wake {
            self.wake.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_requests_rerun_for_running_titles() {
        let queue = PostHydrationTitleScanQueue::new();
        let token = CancellationToken::new();

        assert!(queue.enqueue("title-1".to_string()).await);
        let first = queue
            .dequeue(&token)
            .await
            .expect("first dequeue should succeed");
        assert_eq!(first, "title-1");

        assert!(!queue.enqueue("title-1".to_string()).await);
        queue.finish("title-1").await;

        let rerun = queue
            .dequeue(&token)
            .await
            .expect("rerun dequeue should succeed");
        assert_eq!(rerun, "title-1");
    }
}
