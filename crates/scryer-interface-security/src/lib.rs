use std::time::Instant;

use async_graphql::{Context, ID, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{AppError, LoginFailureTimingClass};
use scryer_domain::AppPermission;

use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, require_config_app_permission,
    to_gql_error, to_login_gql_error_after_timing,
};
use scryer_interface_media::mappers::{from_linked_account, from_user_with_auth_factor_status};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct UserMutations;

fn emby_connection_mode(value: EmbyConnectionModeValue) -> scryer_application::EmbyConnectionMode {
    match value {
        EmbyConnectionModeValue::Local => scryer_application::EmbyConnectionMode::Local,
        EmbyConnectionModeValue::Connect => scryer_application::EmbyConnectionMode::Connect,
    }
}

async fn user_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
) -> GqlResult<UserPayload> {
    let auth_factor_status = app
        .user_auth_factor_status(&user.id)
        .await
        .map_err(to_gql_error)?;
    Ok(from_user_with_auth_factor_status(user, auth_factor_status))
}

async fn login_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    mfa_verified_until: Option<chrono::DateTime<Utc>>,
    mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
    persist_session: bool,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_access_token_with_mfa(&user, mfa_verified_until, mfa_step_up_verified_until)
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(app.token_lifetime());
    Ok(LoginPayload {
        token,
        user: user_payload_from_user(app, user).await?,
        expires_at,
        mfa_verified_until,
        mfa_enrollment_required: false,
        persist_session,
    })
}

async fn login_mfa_enrollment_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    persist_session: bool,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_mfa_enrollment_token(&user, persist_session)
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(app.mfa_enrollment_token_lifetime());
    Ok(LoginPayload {
        token,
        user: user_payload_from_user(app, user).await?,
        expires_at,
        mfa_verified_until: None,
        mfa_enrollment_required: true,
        persist_session,
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
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
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
                )
                .normalized_for_storage();
                scryer_domain::LibraryGrant {
                    user_id: String::new(),
                    library_id: String::from(grant.library_id),
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
        user_payload_from_user(&app, user).await
    }

    async fn set_user_password(
        &self,
        ctx: &Context<'_>,
        input: SetUserPasswordInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user_id = String::from(input.user_id);
        let actor = if user_id != actor.id {
            require_config_app_permission(ctx, AppPermission::ManageUsers).await?
        } else {
            actor
        };
        let user = if user_id == actor.id {
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
            app.set_user_password(&actor, &user_id, input.password)
                .await
                .map_err(to_gql_error)?
        };
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    async fn set_user_app_permissions(
        &self,
        ctx: &Context<'_>,
        input: SetUserAppPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManagePermissions).await?;
        let user_id = String::from(input.user_id);
        let permissions = scryer_domain::AppPermissionMask::from_permissions(
            input
                .permissions
                .into_iter()
                .map(|permission| permission.into_domain()),
        );
        let user = app
            .set_user_app_permissions(&actor, &user_id, permissions)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    async fn set_user_library_permissions(
        &self,
        ctx: &Context<'_>,
        input: SetUserLibraryPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManagePermissions).await?;
        let user_id = String::from(input.user_id);
        let grants = input
            .grants
            .into_iter()
            .map(|grant| {
                let permissions = scryer_domain::LibraryPermissionMask::from_permissions(
                    grant
                        .permissions
                        .into_iter()
                        .map(|permission| permission.into_domain()),
                )
                .normalized_for_storage();
                scryer_domain::LibraryGrant {
                    user_id: user_id.clone(),
                    library_id: String::from(grant.library_id),
                    permissions,
                }
            })
            .collect();
        let user = app
            .set_user_library_permissions(&actor, &user_id, grants)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    async fn set_user_login_enabled(
        &self,
        ctx: &Context<'_>,
        input: SetUserLoginEnabledInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let user = app
            .set_user_login_enabled(
                &actor,
                input.user_id.as_str(),
                input.enabled,
                auth_runtime.snapshot().effective_form_login_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        auth_runtime.invalidate_connections();
        user_payload_from_user(&app, user).await
    }

    async fn delete_user(&self, ctx: &Context<'_>, id: ID) -> GqlResult<DeleteUserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let id_string = id.to_string();
        app.delete_user(&actor, &id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteUserPayload { id })
    }

    async fn reset_user_mfa(&self, ctx: &Context<'_>, id: ID) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let id = id.to_string();
        let user = app
            .reset_user_mfa(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    async fn create_external_account_invite(
        &self,
        ctx: &Context<'_>,
        input: CreateExternalAccountInviteInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let user_id = input.user_id.to_string();
        let connection_id = input.connection_id.to_string();
        app.create_external_account_invite(
            &actor,
            &user_id,
            input.provider.into_domain(),
            connection_id,
            input.provider_user_identifier,
            input.provider_user_id,
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
        app.link_plex_account(
            &actor,
            input.connection_id.to_string(),
            input.plex_auth_token,
        )
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
        app.link_jellyfin_account(
            &actor,
            input.connection_id.to_string(),
            input.username,
            input.password,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    async fn link_emby_account(
        &self,
        ctx: &Context<'_>,
        input: LinkEmbyAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_emby_account(
            &actor,
            input.connection_id.to_string(),
            emby_connection_mode(input.mode),
            input.username,
            input.password,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    async fn unlink_external_account(
        &self,
        ctx: &Context<'_>,
        linked_account_id: ID,
    ) -> GqlResult<UnlinkExternalAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let linked_account_id_string = linked_account_id.to_string();
        app.unlink_external_account(&actor, &linked_account_id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(UnlinkExternalAccountPayload { linked_account_id })
    }

    async fn login_with_plex(
        &self,
        ctx: &Context<'_>,
        input: LoginWithPlexInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = input.persist_session.unwrap_or(true);
        let user = match app
            .federated_login_with_plex(input.connection_id.to_string(), input.plex_auth_token)
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "plex",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        login_payload_from_user(&app, user, None, None, persist_session).await
    }

    async fn login_with_jellyfin(
        &self,
        ctx: &Context<'_>,
        input: LoginWithJellyfinInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = input.persist_session.unwrap_or(true);
        let user = match app
            .federated_login_with_jellyfin(
                input.connection_id.to_string(),
                input.username,
                input.password,
            )
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "jellyfin",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
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
                return login_mfa_enrollment_payload_from_user(&app, user, persist_session).await;
            }
            let code = input.totp_code.as_deref().ok_or_else(|| {
                to_gql_error(AppError::MfaStepUpRequired(
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
        login_payload_from_user(&app, user, mfa_verified_until, None, persist_session).await
    }

    async fn login_with_emby(
        &self,
        ctx: &Context<'_>,
        input: LoginWithEmbyInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = input.persist_session.unwrap_or(true);
        let user = match app
            .federated_login_with_emby(
                input.connection_id.to_string(),
                emby_connection_mode(input.mode),
                input.username,
                input.password,
            )
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "emby",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        let emby_mfa_required = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .totp_require_emby_login;
        let mfa_verified_until = if emby_mfa_required {
            if !app.totp_status(&user).await.map_err(to_gql_error)?.enabled {
                return login_mfa_enrollment_payload_from_user(&app, user, persist_session).await;
            }
            let code = input.totp_code.as_deref().ok_or_else(|| {
                to_gql_error(AppError::MfaStepUpRequired(
                    "TOTP code is required for Emby login".into(),
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
        login_payload_from_user(&app, user, mfa_verified_until, None, persist_session).await
    }
}
