use async_graphql::{Context, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::AppError;

use crate::context::{actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, to_gql_error};
use crate::mappers::{from_linked_account, from_user};
use crate::types::*;

#[derive(Default)]
pub(crate) struct UserMutations;

async fn login_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    mfa_verified_until: Option<chrono::DateTime<Utc>>,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_access_token_with_mfa(&user, mfa_verified_until)
        .await
        .map_err(to_gql_error)?;
    let expires_at = (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
    Ok(LoginPayload {
        token,
        user: from_user(user),
        expires_at,
        mfa_verified_until: mfa_verified_until.map(|value| value.to_rfc3339()),
        mfa_enrollment_required: false,
    })
}

async fn login_mfa_enrollment_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_mfa_enrollment_token(&user)
        .await
        .map_err(to_gql_error)?;
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(app.mfa_enrollment_token_lifetime())).to_rfc3339();
    Ok(LoginPayload {
        token,
        user: from_user(user),
        expires_at,
        mfa_verified_until: None,
        mfa_enrollment_required: true,
    })
}

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
            if let Some(current_password) = input.current_password {
                app.change_own_password(&actor, input.password, current_password)
                    .await
                    .map_err(to_gql_error)?
            } else {
                app.set_initial_own_password(&actor, input.password)
                    .await
                    .map_err(to_gql_error)?
            }
        } else {
            app.set_user_password(&actor, &input.user_id, input.password)
                .await
                .map_err(to_gql_error)?
        };
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
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

    async fn create_external_account_invite(
        &self,
        ctx: &Context<'_>,
        input: CreateExternalAccountInviteInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.create_external_account_invite(
            &actor,
            &input.user_id,
            input.provider.into_domain(),
            input.connection_id,
            input.provider_user_identifier,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    async fn link_plex_account(
        &self,
        ctx: &Context<'_>,
        input: LinkPlexAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_plex_account(&actor, input.connection_id, input.plex_auth_token)
            .await
            .map(from_linked_account)
            .map_err(to_gql_error)
    }

    async fn link_jellyfin_account(
        &self,
        ctx: &Context<'_>,
        input: LinkJellyfinAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_jellyfin_account(&actor, input.connection_id, input.username, input.password)
            .await
            .map(from_linked_account)
            .map_err(to_gql_error)
    }

    async fn unlink_external_account(
        &self,
        ctx: &Context<'_>,
        input: UnlinkExternalAccountInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.unlink_external_account(&actor, &input.linked_account_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn login_with_plex(
        &self,
        ctx: &Context<'_>,
        input: LoginWithPlexInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .federated_login_with_plex(input.connection_id, input.plex_auth_token)
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(&app, user, None).await
    }

    async fn login_with_jellyfin(
        &self,
        ctx: &Context<'_>,
        input: LoginWithJellyfinInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .federated_login_with_jellyfin(input.connection_id, input.username, input.password)
            .await
            .map_err(to_gql_error)?;
        let effective_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let jellyfin_mfa_required = effective_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .totp_require_jellyfin_login;
        let mfa_verified_until = if jellyfin_mfa_required {
            if !app.totp_status(&user).await.map_err(to_gql_error)?.enabled {
                return login_mfa_enrollment_payload_from_user(&app, user).await;
            }
            let code = input.totp_code.as_deref().ok_or_else(|| {
                to_gql_error(AppError::TotpStepUpRequired(
                    "TOTP code is required for Jellyfin login".into(),
                ))
            })?;
            Some(
                app.verify_totp_for_user(&user, code)
                    .await
                    .map_err(to_gql_error)?,
            )
        } else {
            None
        };
        login_payload_from_user(&app, user, mfa_verified_until).await
    }
}
