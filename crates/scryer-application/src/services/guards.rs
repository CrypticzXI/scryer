use super::*;

/// In-process guard table for download-submission dedupe and scope ownership.
///
/// Scryer is intentionally single-instance, so the database lookup remains the
/// authoritative duplicate check while this table serializes same-process races.
#[derive(Clone, Default)]
pub struct DownloadSubmissionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
    uncertain_titles: Arc<std::sync::Mutex<HashMap<String, UncertainDownloadSubmissionClaim>>>,
}

#[derive(Clone)]
pub(crate) enum UncertainDownloadSubmissionClaim {
    Accepted {
        submission: DownloadSubmission,
        accepted_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    },
    Ambiguous {
        download_id: scryer_domain::download_identity::DownloadId,
        submission: Option<DownloadSubmission>,
    },
}

impl UncertainDownloadSubmissionClaim {
    pub(crate) fn accepted(
        submission: DownloadSubmission,
        accepted_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    ) -> Self {
        Self::Accepted {
            submission,
            accepted_identity,
            seed_goals,
        }
    }

    pub(crate) fn ambiguous(
        download_id: scryer_domain::download_identity::DownloadId,
        submission: Option<DownloadSubmission>,
    ) -> Self {
        Self::Ambiguous {
            download_id,
            submission,
        }
    }
}

impl DownloadSubmissionGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire_title(&self, title_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.acquire_key(title_id.to_string()).await
    }

    pub(crate) fn mark_uncertain(&self, title_id: &str, claim: UncertainDownloadSubmissionClaim) {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(title_id.to_string(), claim);
    }

    pub(crate) fn clear_uncertain(&self, title_id: &str) {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(title_id);
    }

    pub(crate) fn uncertain_claim(
        &self,
        title_id: &str,
    ) -> Option<UncertainDownloadSubmissionClaim> {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(title_id)
            .cloned()
    }
}

/// In-process guard table for failed-download handling dedupe.
///
/// This serializes same-process races between the grabbed-item failure sweep and
/// tracked-download failure processing while the persisted blocklist row remains
/// the authoritative record of whether failure side effects already ran.
#[derive(Clone, Default)]
pub struct DownloadFailureGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl DownloadFailureGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire(
        &self,
        title_id: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        let key = format!(
            "{title_id}:{}:{}:{}",
            client_id.trim(),
            client_type.trim().to_ascii_lowercase(),
            client_item_id.trim()
        );
        Some(self.acquire_key(key).await)
    }

    pub async fn acquire_release_or_client_item(
        &self,
        title_id: Option<&str>,
        source_title: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        if let Some(source_title) = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        {
            return Some(
                self.acquire_key(format!("release:{title_id}:{source_title}"))
                    .await,
            );
        }

        self.acquire(Some(title_id), client_id, client_type, client_item_id)
            .await
    }
}

#[derive(Clone, Default)]
pub struct BackupExecutionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

pub type InteractiveOperationGuardTable = BackupExecutionGuardTable;

impl BackupExecutionGuardTable {
    async fn lock_for_key(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
            existing
        } else {
            let created = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key, Arc::downgrade(&created));
            created
        }
    }

    pub async fn try_acquire(&self, key: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let lock = self.lock_for_key(key.to_string()).await;
        lock.try_lock_owned().ok()
    }
}

#[derive(Clone, Default)]
pub struct PluginOperationGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl PluginOperationGuardTable {
    pub async fn acquire(&self, plugin_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let key = plugin_id.trim().to_ascii_lowercase();
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }
}
