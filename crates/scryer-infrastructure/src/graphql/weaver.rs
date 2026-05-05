pub(crate) const TEST_CONNECTION_QUERY: &str = include_str!("weaver/test_connection.graphql");
pub(crate) const VERSION_COMPAT_QUERY: &str = include_str!("weaver/version_compat.graphql");
pub(crate) const QUEUE_ITEMS_QUERY: &str = include_str!("weaver/queue_items.graphql");
pub(crate) const HISTORY_ITEMS_QUERY: &str = include_str!("weaver/history_items.graphql");
pub(crate) const JOBS_COMPAT_QUERY: &str = include_str!("weaver/jobs_compat.graphql");
pub(crate) const SUBMIT_NZB_MUTATION: &str = include_str!("weaver/submit_nzb.graphql");
pub(crate) const SUBMIT_NZB_COMPAT_MUTATION: &str =
    include_str!("weaver/submit_nzb_compat.graphql");
pub(crate) const PAUSE_QUEUE_ITEM_MUTATION: &str = include_str!("weaver/pause_queue_item.graphql");
pub(crate) const PAUSE_JOB_MUTATION: &str = include_str!("weaver/pause_job.graphql");
pub(crate) const RESUME_QUEUE_ITEM_MUTATION: &str =
    include_str!("weaver/resume_queue_item.graphql");
pub(crate) const RESUME_JOB_MUTATION: &str = include_str!("weaver/resume_job.graphql");
pub(crate) const REMOVE_HISTORY_ITEMS_MUTATION: &str =
    include_str!("weaver/remove_history_items.graphql");
pub(crate) const DELETE_HISTORY_BATCH_MUTATION: &str =
    include_str!("weaver/delete_history_batch.graphql");
pub(crate) const CANCEL_QUEUE_ITEM_MUTATION: &str =
    include_str!("weaver/cancel_queue_item.graphql");
pub(crate) const CANCEL_JOB_MUTATION: &str = include_str!("weaver/cancel_job.graphql");
