#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySettings {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    pub totp_require_config_step_up: bool,
    pub totp_require_local_login: bool,
    pub totp_require_jellyfin_login: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSecuritySettings {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    pub totp_require_config_step_up: bool,
    pub totp_require_local_login: bool,
    pub totp_require_jellyfin_login: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}
impl AppUseCase {
    async fn load_security_settings(&self) -> AppResult<SecuritySettings> {
        let form_login_enabled = self
            .read_setting_bool_value(FORM_LOGIN_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let password_min_length = self.password_min_length().await?;
        let skip_login_for_local_ips = self
            .read_setting_bool_value(SKIP_LOGIN_FOR_LOCAL_IPS_KEY, None)
            .await?
            .unwrap_or(false);
        let totp_require_config_step_up = self
            .read_setting_bool_value(TOTP_REQUIRE_CONFIG_STEP_UP_KEY, None)
            .await?
            .unwrap_or(false);
        let totp_require_jellyfin_login = self
            .read_setting_bool_value(TOTP_REQUIRE_JELLYFIN_LOGIN_KEY, None)
            .await?
            .unwrap_or(false);
        let totp_require_local_login = self
            .read_setting_bool_value(TOTP_REQUIRE_LOCAL_LOGIN_KEY, None)
            .await?
            .unwrap_or(false);

        Ok(SecuritySettings {
            form_login_enabled,
            password_min_length,
            skip_login_for_local_ips,
            totp_require_config_step_up,
            totp_require_local_login,
            totp_require_jellyfin_login,
        })
    }
}
impl AppUseCase {
    pub async fn security_settings(&self) -> AppResult<SecuritySettings> {
        self.load_security_settings().await
    }
}
impl AppUseCase {
    pub async fn get_security_settings(&self, actor: &User) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.load_security_settings().await
    }
}
impl AppUseCase {
    pub async fn setup_complete(&self) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(SETUP_COMPLETE_KEY, None)
            .await?
            .unwrap_or(false))
    }
}
impl AppUseCase {
    pub async fn complete_setup(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                SETUP_COMPLETE_KEY,
                None,
                encode_setting_json(&true)?,
                "setup-wizard",
                Some(actor.id.clone()),
            )
            .await?;

        Ok(true)
    }
}
impl AppUseCase {
    pub async fn get_service_settings(&self, actor: &User) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(ServiceSettings {
            tls_cert_path: self
                .read_setting_string_value(TLS_CERT_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
            tls_key_path: self
                .read_setting_string_value(TLS_KEY_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
        })
    }
}
impl AppUseCase {
    pub async fn update_security_settings(
        &self,
        actor: &User,
        input: UpdateSecuritySettings,
    ) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let current = self.load_security_settings().await?;

        if input.password_min_length < PASSWORD_MIN_LENGTH_MIN as i32 {
            return Err(AppError::Validation(format!(
                "password minimum length must be at least {PASSWORD_MIN_LENGTH_MIN}"
            )));
        }

        if input.totp_require_config_step_up
            && self
                .services
                .identity
                .totp
                .get_credential_for_user(&actor.id)
                .await?
                .is_none()
        {
            return Err(AppError::TotpEnrollmentRequired(
                "enable TOTP for your account before requiring TOTP for system configuration"
                    .into(),
            ));
        }

        if !current.form_login_enabled
            && input.form_login_enabled
            && self.default_admin_uses_bootstrap_password().await?
        {
            return Err(AppError::Validation(
                "change the default admin password before enabling form login".into(),
            ));
        }

        self.upsert_system_setting_json(
            FORM_LOGIN_ENABLED_KEY,
            &input.form_login_enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            PASSWORD_MIN_LENGTH_KEY,
            &input.password_min_length,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SKIP_LOGIN_FOR_LOCAL_IPS_KEY,
            &input.skip_login_for_local_ips,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            TOTP_REQUIRE_CONFIG_STEP_UP_KEY,
            &input.totp_require_config_step_up,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            TOTP_REQUIRE_JELLYFIN_LOGIN_KEY,
            &input.totp_require_jellyfin_login,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            TOTP_REQUIRE_LOCAL_LOGIN_KEY,
            &input.totp_require_local_login,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_settings_saved(
            actor,
            "security_settings",
            None,
            vec![
                FORM_LOGIN_ENABLED_KEY.to_string(),
                PASSWORD_MIN_LENGTH_KEY.to_string(),
                SKIP_LOGIN_FOR_LOCAL_IPS_KEY.to_string(),
                TOTP_REQUIRE_CONFIG_STEP_UP_KEY.to_string(),
                TOTP_REQUIRE_LOCAL_LOGIN_KEY.to_string(),
                TOTP_REQUIRE_JELLYFIN_LOGIN_KEY.to_string(),
            ],
        )
        .await;

        Ok(SecuritySettings {
            form_login_enabled: input.form_login_enabled,
            password_min_length: input.password_min_length,
            skip_login_for_local_ips: input.skip_login_for_local_ips,
            totp_require_config_step_up: input.totp_require_config_step_up,
            totp_require_local_login: input.totp_require_local_login,
            totp_require_jellyfin_login: input.totp_require_jellyfin_login,
        })
    }
}
impl AppUseCase {
    pub async fn update_service_settings(
        &self,
        actor: &User,
        input: UpdateServiceSettings,
    ) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let tls_cert_path = input.tls_cert_path.trim().to_string();
        let tls_key_path = input.tls_key_path.trim().to_string();

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_CERT_PATH_KEY,
                None,
                encode_setting_json(&tls_cert_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_KEY_PATH_KEY,
                None,
                encode_setting_json(&tls_key_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "service_settings",
            None,
            vec![TLS_CERT_PATH_KEY.to_string(), TLS_KEY_PATH_KEY.to_string()],
        )
        .await;

        self.get_service_settings(actor).await
    }
}
