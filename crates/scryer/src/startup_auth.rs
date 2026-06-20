use scryer_application::AppUseCase;

pub(crate) async fn ensure_admin_password_configured(
    app_use_case: &AppUseCase,
) -> Result<(), String> {
    if app_use_case
        .existing_default_admin_uses_bootstrap_password()
        .await
        .map_err(|error| format!("failed to validate default admin password state: {error}"))?
    {
        return Err(
            "form login is enabled, but the default admin password is still 'admin'; change it before enabling auth".to_string(),
        );
    }

    if !app_use_case
        .usable_admin_login_exists()
        .await
        .map_err(|error| format!("failed to validate admin login state: {error}"))?
    {
        return Err(
            "form login is enabled, but no local full-admin user has a usable password; start with SCRYER_RECOVERY_ADMIN_PASSWORD set to recover the instance".to_string(),
        );
    }

    Ok(())
}
