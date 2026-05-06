use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::AppError;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_user;
use crate::types::*;

#[derive(Default)]
pub(crate) struct UserMutations;

#[Object]
impl UserMutations {
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        input: CreateUserInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let app_permissions = scryer_domain::AppPermissionMask::from_permissions(
            input
                .app_permissions
                .into_iter()
                .map(|permission| permission.into_domain()),
        );
        let library_grants = input
            .library_permissions
            .into_iter()
            .map(|grant| {
                let permissions = scryer_domain::LibraryPermissionMask::from_permissions(
                    grant
                        .permissions
                        .into_iter()
                        .map(|permission| permission.into_domain()),
                );
                scryer_domain::LibraryGrant {
                    user_id: String::new(),
                    library_id: grant.library_id,
                    permissions,
                }
            })
            .collect();
        let user = app
            .create_user(
                &actor,
                input.username,
                input.password,
                app_permissions,
                library_grants,
            )
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn set_user_password(
        &self,
        ctx: &Context<'_>,
        input: SetUserPasswordInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user = if input.user_id == actor.id {
            let current_password = input.current_password.ok_or_else(|| {
                to_gql_error(AppError::Validation("current password is required".into()))
            })?;
            app.change_own_password(&actor, input.password, current_password)
                .await
                .map_err(to_gql_error)?
        } else {
            app.set_user_password(&actor, &input.user_id, input.password)
                .await
                .map_err(to_gql_error)?
        };
        Ok(from_user(user))
    }

    async fn set_user_app_permissions(
        &self,
        ctx: &Context<'_>,
        input: SetUserAppPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let permissions = scryer_domain::AppPermissionMask::from_permissions(
            input
                .permissions
                .into_iter()
                .map(|permission| permission.into_domain()),
        );
        let user = app
            .set_user_app_permissions(&actor, &input.user_id, permissions)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn set_user_library_permissions(
        &self,
        ctx: &Context<'_>,
        input: SetUserLibraryPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let grants = input
            .grants
            .into_iter()
            .map(|grant| {
                let permissions = scryer_domain::LibraryPermissionMask::from_permissions(
                    grant
                        .permissions
                        .into_iter()
                        .map(|permission| permission.into_domain()),
                );
                scryer_domain::LibraryGrant {
                    user_id: input.user_id.clone(),
                    library_id: grant.library_id,
                    permissions,
                }
            })
            .collect();
        let user = app
            .set_user_library_permissions(&actor, &input.user_id, grants)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn delete_user(&self, ctx: &Context<'_>, input: DeleteUserInput) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_user(&actor, &input.user_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }
}
