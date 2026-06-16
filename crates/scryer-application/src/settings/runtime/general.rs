const SMG_VERSION_COMPATIBILITY_NOTICE_KEY: &str = "smg.version_compatibility_notice";
const SMG_SCRYER_UPDATE_NOTICE_KEY: &str = "smg.scryer_update_notice";
fn normalize_auto_backup_daily_time_local(value: &str) -> AppResult<String> {
    let value = value.trim();
    let (hour, minute) = value
        .split_once(':')
        .ok_or_else(|| AppError::Validation("daily time must use HH:MM format".to_string()))?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time hour must be numeric".to_string()))?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time minute must be numeric".to_string()))?;
    if hour > 23 || minute > 59 {
        return Err(AppError::Validation(
            "daily time must be between 00:00 and 23:59".to_string(),
        ));
    }
    Ok(format!("{hour:02}:{minute:02}"))
}
fn validate_auto_backup_key_update(
    set_auto_backup_key: Option<&str>,
    clear_auto_backup_key: bool,
) -> AppResult<()> {
    if clear_auto_backup_key && set_auto_backup_key.is_some_and(|value| !value.is_empty()) {
        return Err(AppError::Validation(
            "automatic backup key cannot be replaced and cleared in the same request".to_string(),
        ));
    }

    Ok(())
}
fn split_pem_certificate_blocks(bundle_pem: &str) -> AppResult<Vec<String>> {
    let trimmed = bundle_pem.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    let blocks = PEM_CERT_BLOCK_RE
        .find_iter(trimmed)
        .map(|matched| {
            let block = matched.as_str().trim();
            if block.ends_with('\n') {
                block.to_string()
            } else {
                format!("{block}\n")
            }
        })
        .collect::<Vec<_>>();

    if blocks.is_empty() {
        return Err(AppError::Validation(
            "trusted certificate bundle must contain PEM-encoded X.509 certificates".to_string(),
        ));
    }

    let remaining = PEM_CERT_BLOCK_RE.replace_all(trimmed, "");
    if !remaining.trim().is_empty() {
        return Err(AppError::Validation(
            "trusted certificate bundle may only contain X.509 certificate PEM blocks".to_string(),
        ));
    }

    Ok(blocks)
}
fn parse_pem_certificate_der(block_pem: &str) -> AppResult<Vec<u8>> {
    let mut cursor = Cursor::new(block_pem.as_bytes());
    match read_one(&mut cursor).map_err(|error| {
        AppError::Validation(format!(
            "failed to parse trusted certificate PEM block: {error}"
        ))
    })? {
        Some(Item::X509Certificate(cert)) => {
            if read_one(&mut cursor)
                .map_err(|error| {
                    AppError::Validation(format!(
                        "failed to parse trailing PEM content for trusted certificate: {error}"
                    ))
                })?
                .is_some()
            {
                return Err(AppError::Validation(
                    "each trusted certificate entry must contain exactly one X.509 certificate"
                        .to_string(),
                ));
            }
            Ok(cert.as_ref().to_vec())
        }
        Some(_) => Err(AppError::Validation(
            "trusted certificate bundle may only contain X.509 certificates".to_string(),
        )),
        None => Err(AppError::Validation(
            "trusted certificate bundle did not contain a readable X.509 certificate".to_string(),
        )),
    }
}
fn normalize_plugin_http_ca_bundle_pem(bundle_pem: &str) -> AppResult<String> {
    let blocks = split_pem_certificate_blocks(bundle_pem)?;
    if blocks.is_empty() {
        return Ok(String::new());
    }

    let mut normalized = Vec::with_capacity(blocks.len());
    for block in blocks {
        let _ = parse_pem_certificate_der(&block)?;
        normalized.push(block);
    }
    Ok(normalized.join("\n"))
}
fn summarize_plugin_http_trusted_certificates(
    bundle_pem: &str,
) -> AppResult<Vec<GeneralSettingsTrustedCertificate>> {
    let blocks = split_pem_certificate_blocks(bundle_pem)?;
    let mut certificates = Vec::with_capacity(blocks.len());
    for block in blocks {
        let der = parse_pem_certificate_der(&block)?;
        let digest = aws_lc_digest::digest(&aws_lc_digest::SHA256, &der);
        certificates.push(GeneralSettingsTrustedCertificate {
            fingerprint_sha256: crate::helpers::to_hex(digest.as_ref()),
            pem: block,
        });
    }
    Ok(certificates)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralSettings {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
    pub plugin_http_ca_bundle_pem: String,
    pub plugin_http_trusted_certificates: Vec<GeneralSettingsTrustedCertificate>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralSettingsTrustedCertificate {
    pub fingerprint_sha256: String,
    pub pem: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub auto_backup_key_present: bool,
    pub next_run_at: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateGeneralSettings {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
    pub plugin_http_ca_bundle_pem: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub set_auto_backup_key: Option<String>,
    pub clear_auto_backup_key: bool,
}
impl AppUseCase {
    async fn load_general_settings(&self) -> AppResult<GeneralSettings> {
        let keep_history_forever = self
            .read_setting_bool_value(HISTORY_KEEP_FOREVER_KEY, None)
            .await?
            .unwrap_or(false);
        let history_retention_days = self
            .read_setting_i64_value(HISTORY_RETENTION_DAYS_KEY, None)
            .await?
            .map(|value| value.max(1) as i32)
            .unwrap_or(180);
        let stored_bundle = self
            .read_setting_string_value(PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, None)
            .await?
            .unwrap_or_default();
        let (plugin_http_ca_bundle_pem, plugin_http_trusted_certificates) =
            match normalize_plugin_http_ca_bundle_pem(&stored_bundle).and_then(|bundle| {
                let certificates = summarize_plugin_http_trusted_certificates(&bundle)?;
                Ok((bundle, certificates))
            }) {
                Ok(result) => result,
                Err(error) => {
                    if !stored_bundle.trim().is_empty() {
                        warn!(
                            error = %error,
                            "stored plugin HTTP trusted certificate bundle could not be normalized"
                        );
                    }
                    (stored_bundle, Vec::new())
                }
            };

        Ok(GeneralSettings {
            keep_history_forever,
            history_retention_days,
            plugin_http_ca_bundle_pem,
            plugin_http_trusted_certificates,
        })
    }
}
impl AppUseCase {
    pub(crate) async fn load_auto_backup_settings(&self) -> AppResult<AutoBackupSettings> {
        let enabled = self
            .read_setting_bool_value(AUTO_BACKUP_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let daily_time_local = normalize_auto_backup_daily_time_local(
            &self
                .read_setting_string_value(AUTO_BACKUP_DAILY_TIME_LOCAL_KEY, None)
                .await?
                .unwrap_or_else(|| DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL.to_string()),
        )?;
        let auto_backup_key_present = self
            .read_setting_string_value(AUTO_BACKUP_KEY_KEY, None)
            .await?
            .is_some_and(|value| !value.is_empty());
        let next_run_at = if enabled {
            Some(
                crate::security::backup::compute_next_auto_backup_run_at(
                    &daily_time_local,
                    chrono::Utc::now(),
                )?
                .to_rfc3339(),
            )
        } else {
            None
        };

        Ok(AutoBackupSettings {
            enabled,
            daily_time_local,
            auto_backup_key_present,
            next_run_at,
        })
    }
}
impl AppUseCase {
    pub(crate) async fn general_settings(&self) -> AppResult<GeneralSettings> {
        self.load_general_settings().await
    }
}
impl AppUseCase {
    pub async fn get_general_settings(&self, actor: &User) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_general_settings().await
    }
}
impl AppUseCase {
    pub async fn get_auto_backup_settings(&self, actor: &User) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_auto_backup_settings().await
    }
}
impl AppUseCase {
    pub async fn update_general_settings(
        &self,
        actor: &User,
        input: UpdateGeneralSettings,
    ) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let current = self.load_general_settings().await?;
        let history_retention_days =
            if input.keep_history_forever && input.history_retention_days < 1 {
                current.history_retention_days
            } else {
                input.history_retention_days
            };

        if history_retention_days < 1 {
            return Err(AppError::Validation(
                "history retention days must be at least 1".to_string(),
            ));
        }
        let plugin_http_ca_bundle_pem =
            normalize_plugin_http_ca_bundle_pem(&input.plugin_http_ca_bundle_pem)?;
        let plugin_http_trusted_certificates =
            summarize_plugin_http_trusted_certificates(&plugin_http_ca_bundle_pem)?;

        self.upsert_system_setting_json(
            HISTORY_KEEP_FOREVER_KEY,
            &input.keep_history_forever,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            HISTORY_RETENTION_DAYS_KEY,
            &history_retention_days,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            PLUGIN_HTTP_CA_BUNDLE_PEM_KEY,
            &plugin_http_ca_bundle_pem,
            Some(actor.id.clone()),
        )
        .await?;
        if let Some(runtime) = self.services.config.plugin_http_trust_runtime.available() {
            runtime.set_plugin_http_ca_bundle_pem(plugin_http_ca_bundle_pem.clone())?;
        }

        self.emit_settings_saved(
            actor,
            "general_settings",
            None,
            vec![
                HISTORY_KEEP_FOREVER_KEY.to_string(),
                HISTORY_RETENTION_DAYS_KEY.to_string(),
                PLUGIN_HTTP_CA_BUNDLE_PEM_KEY.to_string(),
            ],
        )
        .await;

        Ok(GeneralSettings {
            keep_history_forever: input.keep_history_forever,
            history_retention_days,
            plugin_http_ca_bundle_pem,
            plugin_http_trusted_certificates,
        })
    }
}
impl AppUseCase {
    pub async fn update_auto_backup_settings(
        &self,
        actor: &User,
        input: UpdateAutoBackupSettings,
    ) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        validate_auto_backup_key_update(
            input.set_auto_backup_key.as_deref(),
            input.clear_auto_backup_key,
        )?;
        let daily_time_local = normalize_auto_backup_daily_time_local(&input.daily_time_local)?;

        self.upsert_system_setting_json(
            AUTO_BACKUP_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
            &daily_time_local,
            Some(actor.id.clone()),
        )
        .await?;

        let mut changed_keys = vec![
            AUTO_BACKUP_ENABLED_KEY.to_string(),
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY.to_string(),
        ];

        if input.clear_auto_backup_key {
            self.delete_system_setting(AUTO_BACKUP_KEY_KEY).await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        } else if let Some(set_auto_backup_key) = input.set_auto_backup_key
            && !set_auto_backup_key.is_empty()
        {
            self.upsert_system_setting_json(
                AUTO_BACKUP_KEY_KEY,
                &set_auto_backup_key,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        }

        self.emit_settings_saved(actor, "auto_backup_settings", None, changed_keys)
            .await;

        self.load_auto_backup_settings().await
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );

    #[test]
    fn normalize_auto_backup_daily_time_local_trims_and_zero_pads_values() {
        let normalized = normalize_auto_backup_daily_time_local(" 3:5 ").expect("normalized time");

        assert_eq!(normalized, "03:05");
    }

    #[test]
    fn normalize_auto_backup_daily_time_local_rejects_invalid_values() {
        assert!(normalize_auto_backup_daily_time_local("24:00").is_err());
        assert!(normalize_auto_backup_daily_time_local("10:60").is_err());
        assert!(normalize_auto_backup_daily_time_local("nope").is_err());
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_replace_and_clear_together() {
        let error = validate_auto_backup_key_update(Some("secret"), true)
            .expect_err("set and clear should be rejected");

        assert!(
            error
                .to_string()
                .contains("automatic backup key cannot be replaced and cleared"),
        );
    }

    #[test]
    fn normalize_plugin_http_ca_bundle_pem_rejects_trailing_non_certificate_text() {
        let error = normalize_plugin_http_ca_bundle_pem(&format!(
            "{TEST_PLUGIN_HTTP_CA_CERT_PEM}\nnot-a-certificate"
        ))
        .expect_err("trailing text should be rejected");

        assert!(
            error
                .to_string()
                .contains("may only contain X.509 certificate PEM blocks"),
        );
    }

    #[test]
    fn summarize_plugin_http_trusted_certificates_preserves_normalized_blocks() {
        let normalized = normalize_plugin_http_ca_bundle_pem(&format!(
            "{TEST_PLUGIN_HTTP_CA_CERT_PEM}\n\n{TEST_PLUGIN_HTTP_CA_CERT_PEM}"
        ))
        .expect("normalized certificate bundle");
        let certificates = summarize_plugin_http_trusted_certificates(&normalized)
            .expect("summarized certificate bundle");

        assert_eq!(certificates.len(), 2);
        assert_eq!(
            certificates[0].fingerprint_sha256,
            certificates[1].fingerprint_sha256
        );
        assert!(!certificates[0].fingerprint_sha256.is_empty());
        assert_eq!(certificates[0].pem, TEST_PLUGIN_HTTP_CA_CERT_PEM);
        assert_eq!(certificates[1].pem, TEST_PLUGIN_HTTP_CA_CERT_PEM);
    }
}
