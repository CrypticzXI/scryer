use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Duration, Utc};

use super::*;

const TOTP_ALGORITHM: &str = "SHA256";
const TOTP_DIGITS: i32 = 8;
const TOTP_PERIOD_SECONDS: i32 = 30;
const TOTP_SECRET_BYTES: usize = 32;
const TOTP_ENROLLMENT_TTL_MINUTES: i64 = 10;
const TOTP_STEP_UP_TTL_MINUTES: i64 = 60;
const TOTP_ALLOWED_DRIFT_STEPS: i64 = 1;
const TOTP_FAILED_ATTEMPT_LIMIT: i64 = 5;
const TOTP_FAILED_ATTEMPT_WINDOW_MINUTES: i64 = 5;
const TOTP_RECOVERY_CODE_COUNT: usize = 10;
const TOTP_RECOVERY_CODE_BYTES: usize = 16;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

impl AppUseCase {
    pub async fn totp_status(&self, actor: &User) -> AppResult<TotpStatus> {
        let credential = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?;
        self.totp_status_from_credential(actor, credential).await
    }

    pub async fn totp_enrollment_start(&self, actor: &User) -> AppResult<TotpEnrollmentStart> {
        self.cleanup_expired_totp_enrollment_challenges().await?;

        if self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation("TOTP is already enabled".into()));
        }

        let now = Utc::now();
        let secret_base32 = generate_base32_secret(TOTP_SECRET_BYTES)?;
        let challenge = TotpEnrollmentChallengeRecord {
            id: Id::new().0,
            user_id: actor.id.clone(),
            secret_base32: secret_base32.clone(),
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::minutes(TOTP_ENROLLMENT_TTL_MINUTES)).to_rfc3339(),
        };
        self.services
            .identity
            .totp
            .create_enrollment_challenge(challenge.clone())
            .await?;

        Ok(TotpEnrollmentStart {
            challenge_id: challenge.id,
            otpauth_url: totp_otpauth_url(&actor.username, &secret_base32),
            secret_base32,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn totp_enrollment_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.cleanup_expired_totp_enrollment_challenges().await?;

        let challenge = self
            .services
            .identity
            .totp
            .get_enrollment_challenge(challenge_id, &actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("TOTP challenge {challenge_id}")))?;

        if timestamp_expired(&challenge.expires_at) {
            self.services
                .identity
                .totp
                .delete_enrollment_challenge(challenge_id, &actor.id)
                .await?;
            return Err(AppError::Validation(
                "TOTP enrollment challenge has expired".into(),
            ));
        }

        let normalized_code = normalize_totp_code(code)?;
        let secret = base32_decode(&challenge.secret_base32)?;
        let now = Utc::now();
        let Some(step) = matching_totp_step(&secret, &normalized_code, now)? else {
            return Err(AppError::TotpInvalidCode(
                "TOTP code did not match the enrollment secret".into(),
            ));
        };

        let credential = TotpCredentialRecord {
            id: Id::new().0,
            user_id: actor.id.clone(),
            secret_base32: challenge.secret_base32.clone(),
            algorithm: TOTP_ALGORITHM.to_string(),
            digits: TOTP_DIGITS,
            period_seconds: TOTP_PERIOD_SECONDS,
            last_accepted_step: Some(step),
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            last_used_at: Some(now.to_rfc3339()),
        };
        self.services
            .identity
            .totp
            .upsert_credential(credential.clone())
            .await?;
        self.services
            .identity
            .totp
            .delete_enrollment_challenge(challenge_id, &actor.id)
            .await?;

        let recovery_codes = self.replace_totp_recovery_codes(actor).await?;
        let status = self
            .totp_status_from_credential(actor, Some(credential))
            .await?;
        Ok(TotpEnrollmentComplete {
            status,
            recovery_codes,
        })
    }

    pub async fn totp_verify_step_up(&self, actor: &User, code: &str) -> AppResult<DateTime<Utc>> {
        self.verify_totp_for_user(actor, code).await
    }

    pub async fn totp_disable(&self, actor: &User, code: &str) -> AppResult<TotpStatus> {
        self.verify_totp_for_user(actor, code).await?;
        self.services
            .identity
            .totp
            .delete_credential_for_user(&actor.id)
            .await?;
        self.services
            .identity
            .totp
            .replace_recovery_codes(&actor.id, Vec::new())
            .await?;
        self.services
            .identity
            .totp
            .clear_failed_attempts(&actor.id)
            .await?;
        self.totp_status(actor).await
    }

    pub async fn totp_regenerate_recovery_codes(
        &self,
        actor: &User,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.verify_totp_for_user(actor, code).await?;
        let recovery_codes = self.replace_totp_recovery_codes(actor).await?;
        let status = self.totp_status(actor).await?;
        Ok(TotpEnrollmentComplete {
            status,
            recovery_codes,
        })
    }

    pub async fn require_totp_step_up(
        &self,
        actor: &User,
        mfa_verified_until: Option<i64>,
    ) -> AppResult<()> {
        let settings = self.security_settings().await?;
        if !settings.totp_require_config_step_up {
            return Ok(());
        }

        let credential = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?;
        if credential.is_none() {
            return Err(AppError::TotpEnrollmentRequired(
                "TOTP enrollment is required before changing system configuration".into(),
            ));
        }

        if mfa_verified_until.is_some_and(|expires_at| expires_at > Utc::now().timestamp()) {
            return Ok(());
        }

        Err(AppError::TotpStepUpRequired(
            "TOTP verification is required before changing system configuration".into(),
        ))
    }

    pub async fn verify_totp_for_user(&self, actor: &User, code: &str) -> AppResult<DateTime<Utc>> {
        let Some(mut credential) = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?
        else {
            return Err(AppError::TotpEnrollmentRequired(
                "TOTP enrollment is required".into(),
            ));
        };

        self.ensure_totp_attempt_allowed(&actor.id).await?;
        match self
            .verify_totp_or_recovery_code(&mut credential, code)
            .await
        {
            Ok(()) => {
                self.services
                    .identity
                    .totp
                    .clear_failed_attempts(&actor.id)
                    .await?;
                Ok(Utc::now() + Duration::minutes(TOTP_STEP_UP_TTL_MINUTES))
            }
            Err(error) => {
                self.record_totp_failed_attempt(&actor.id).await?;
                Err(error)
            }
        }
    }

    async fn verify_totp_or_recovery_code(
        &self,
        credential: &mut TotpCredentialRecord,
        code: &str,
    ) -> AppResult<()> {
        if let Ok(normalized_code) = normalize_totp_code(code) {
            let secret = base32_decode(&credential.secret_base32)?;
            let now = Utc::now();
            if let Some(step) = matching_totp_step(&secret, &normalized_code, now)?
                && credential
                    .last_accepted_step
                    .is_none_or(|last_step| step > last_step)
            {
                credential.last_accepted_step = Some(step);
                credential.last_used_at = Some(now.to_rfc3339());
                credential.updated_at = now.to_rfc3339();
                self.services
                    .identity
                    .totp
                    .upsert_credential(credential.clone())
                    .await?;
                return Ok(());
            }
        }

        self.verify_totp_recovery_code(&credential.user_id, code)
            .await
    }

    async fn verify_totp_recovery_code(&self, user_id: &str, code: &str) -> AppResult<()> {
        let normalized = normalize_recovery_code(code);
        if normalized.is_empty() {
            return Err(AppError::TotpInvalidCode("invalid TOTP code".into()));
        }

        let recovery_codes = self
            .services
            .identity
            .totp
            .list_recovery_codes_for_user(user_id)
            .await?;
        for recovery_code in recovery_codes {
            if self.validate_password(&normalized, &recovery_code.code_hash)? {
                if recovery_code.used_at.is_some() {
                    return Err(AppError::TotpRecoveryCodeUsed(
                        "TOTP recovery code was already used".into(),
                    ));
                }
                self.services
                    .identity
                    .totp
                    .mark_recovery_code_used(&recovery_code.id, user_id, &Utc::now().to_rfc3339())
                    .await?;
                return Ok(());
            }
        }

        Err(AppError::TotpInvalidCode("invalid TOTP code".into()))
    }

    async fn replace_totp_recovery_codes(&self, actor: &User) -> AppResult<Vec<String>> {
        let now = Utc::now().to_rfc3339();
        let mut display_codes = Vec::with_capacity(TOTP_RECOVERY_CODE_COUNT);
        let mut records = Vec::with_capacity(TOTP_RECOVERY_CODE_COUNT);
        for _ in 0..TOTP_RECOVERY_CODE_COUNT {
            let normalized = generate_base32_secret(TOTP_RECOVERY_CODE_BYTES)?;
            let display = group_recovery_code(&normalized);
            records.push(TotpRecoveryCodeRecord {
                id: Id::new().0,
                user_id: actor.id.clone(),
                code_hash: self.hash_password(&normalized)?,
                created_at: now.clone(),
                used_at: None,
            });
            display_codes.push(display);
        }
        self.services
            .identity
            .totp
            .replace_recovery_codes(&actor.id, records)
            .await?;
        Ok(display_codes)
    }

    async fn totp_status_from_credential(
        &self,
        actor: &User,
        credential: Option<TotpCredentialRecord>,
    ) -> AppResult<TotpStatus> {
        let recovery_codes_remaining = self
            .services
            .identity
            .totp
            .list_recovery_codes_for_user(&actor.id)
            .await?
            .into_iter()
            .filter(|code| code.used_at.is_none())
            .count() as i32;
        Ok(TotpStatus {
            enabled: credential.is_some(),
            created_at: credential.as_ref().map(|record| record.created_at.clone()),
            last_used_at: credential.and_then(|record| record.last_used_at),
            recovery_codes_remaining,
        })
    }

    async fn cleanup_expired_totp_enrollment_challenges(&self) -> AppResult<()> {
        self.services
            .identity
            .totp
            .delete_expired_enrollment_challenges(&Utc::now().to_rfc3339())
            .await?;
        Ok(())
    }

    async fn ensure_totp_attempt_allowed(&self, user_id: &str) -> AppResult<()> {
        let since =
            (Utc::now() - Duration::minutes(TOTP_FAILED_ATTEMPT_WINDOW_MINUTES)).to_rfc3339();
        let attempts = self
            .services
            .identity
            .totp
            .count_failed_attempts_since(user_id, &since)
            .await?;
        if attempts >= TOTP_FAILED_ATTEMPT_LIMIT {
            return Err(AppError::TotpInvalidCode(
                "too many invalid TOTP attempts; try again shortly".into(),
            ));
        }
        Ok(())
    }

    async fn record_totp_failed_attempt(&self, user_id: &str) -> AppResult<()> {
        self.services
            .identity
            .totp
            .record_failed_attempt(TotpFailedAttemptRecord {
                id: Id::new().0,
                user_id: user_id.to_string(),
                attempted_at: Utc::now().to_rfc3339(),
            })
            .await
    }
}

fn generate_base32_secret(byte_count: usize) -> AppResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0_u8; byte_count];
    rng.fill(&mut bytes).map_err(|error| {
        AppError::Repository(format!("failed to generate TOTP secret: {error}"))
    })?;
    Ok(base32_encode_no_pad(&bytes))
}

fn matching_totp_step(
    secret: &[u8],
    normalized_code: &str,
    now: DateTime<Utc>,
) -> AppResult<Option<i64>> {
    let current_step = now.timestamp() / i64::from(TOTP_PERIOD_SECONDS);
    for offset in -TOTP_ALLOWED_DRIFT_STEPS..=TOTP_ALLOWED_DRIFT_STEPS {
        let step = current_step + offset;
        if step < 0 {
            continue;
        }
        let expected = hotp_sha256(secret, step as u64, TOTP_DIGITS)?;
        if constant_time_eq(expected.as_bytes(), normalized_code.as_bytes()) {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

fn hotp_sha256(secret: &[u8], counter: u64, digits: i32) -> AppResult<String> {
    if !(6..=10).contains(&digits) {
        return Err(AppError::Repository(format!(
            "unsupported TOTP digit count {digits}"
        )));
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, &counter.to_be_bytes());
    let digest = tag.as_ref();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let modulus = 10_u32.pow(digits as u32);
    Ok(format!(
        "{:0width$}",
        value % modulus,
        width = digits as usize
    ))
}

fn normalize_totp_code(code: &str) -> AppResult<String> {
    let normalized = code
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if normalized.len() != TOTP_DIGITS as usize || !normalized.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(AppError::TotpInvalidCode("invalid TOTP code".into()));
    }
    Ok(normalized)
}

fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn group_recovery_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

fn timestamp_expired(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn totp_otpauth_url(username: &str, secret_base32: &str) -> String {
    let issuer = "Scryer";
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm={}&digits={}&period={}",
        percent_encode_component(issuer),
        percent_encode_component(username),
        secret_base32,
        percent_encode_component(issuer),
        TOTP_ALGORITHM,
        TOTP_DIGITS,
        TOTP_PERIOD_SECONDS
    )
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn base32_encode_no_pad(bytes: &[u8]) -> String {
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0_u16;
    let mut bits_left = 0_u8;

    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits_left += 8;
        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            output.push(char::from(BASE32_ALPHABET[index]));
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        output.push(char::from(BASE32_ALPHABET[index]));
    }

    output
}

fn base32_decode(value: &str) -> AppResult<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0_u32;
    let mut bits_left = 0_u8;

    for ch in value.chars().filter(|ch| *ch != '=') {
        let Some(index) = base32_index(ch) else {
            return Err(AppError::Repository("stored TOTP secret is invalid".into()));
        };
        buffer = (buffer << 5) | u32::from(index);
        bits_left += 5;
        if bits_left >= 8 {
            output.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }

    Ok(output)
}

fn base32_index(ch: char) -> Option<u8> {
    match ch.to_ascii_uppercase() {
        'A'..='Z' => Some(ch.to_ascii_uppercase() as u8 - b'A'),
        '2'..='7' => Some(ch as u8 - b'2' + 26),
        _ => None,
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.iter().zip(right) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_round_trips_without_padding() {
        let bytes = b"hello scryer";
        let encoded = base32_encode_no_pad(bytes);
        assert_eq!(base32_decode(&encoded).unwrap(), bytes);
        assert!(!encoded.contains('='));
    }

    #[test]
    fn hotp_sha256_matches_rfc6238_test_vector() {
        let secret = b"12345678901234567890123456789012";
        assert_eq!(hotp_sha256(secret, 59 / 30, 8).unwrap(), "46119246");
    }

    #[test]
    fn otpauth_url_includes_1password_supported_parameters() {
        let url = totp_otpauth_url("jen@example.test", "JBSWY3DPEHPK3PXP");
        assert_eq!(
            url,
            "otpauth://totp/Scryer:jen%40example.test?secret=JBSWY3DPEHPK3PXP&issuer=Scryer&algorithm=SHA256&digits=8&period=30"
        );
    }
}
