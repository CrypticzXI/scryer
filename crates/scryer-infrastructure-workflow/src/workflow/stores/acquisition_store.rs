use super::*;

use async_trait::async_trait;
use scryer_application::{AcquisitionStateRepository, AppResult, SuccessfulGrabCommit};

use crate::queries::sql_runtime::{SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct AcquisitionStore {
    datastore: StoreDatastore,
}

impl AcquisitionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl AcquisitionStateRepository for AcquisitionStore {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        let commit = commit.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "commit_successful_grab", move |tx| {
            let commit = commit.clone();
            Box::pin(async move { commit_successful_grab_tx(tx, &commit).await })
        })
        .await
    }
}
