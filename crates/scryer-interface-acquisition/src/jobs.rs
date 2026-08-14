use async_graphql::{Context, Object, Result as GqlResult};

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::from_job_run;
use scryer_interface_media::types::{IntoApplication, JobKeyValue, JobRunPayload};

#[derive(Default)]
pub struct JobMutations;

#[Object]
impl JobMutations {
    /// Start the configured background job and return its accepted run snapshot.
    async fn trigger_job(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Job key identifying the server-side job to start.")] job_key: JobKeyValue,
    ) -> GqlResult<JobRunPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let run = app
            .trigger_job(&actor, job_key.into_application())
            .await
            .map_err(to_gql_error)?;
        Ok(from_job_run(run))
    }
}
