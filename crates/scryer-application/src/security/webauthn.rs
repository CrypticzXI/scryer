use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use webauthn_rs::prelude::{
    DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};

use super::*;

const WEBAUTHN_CHALLENGE_TTL_MINUTES: i64 = 5;

#[derive(Debug, Serialize, Deserialize)]
enum StoredAuthenticationState {
    Passkey(PasskeyAuthentication),
    Discoverable(DiscoverableAuthentication),
}

#[derive(Debug, Serialize, Deserialize)]
enum StoredChallengeState {
    Registration(PasskeyRegistration),
    Authentication(StoredAuthenticationState),
}

impl AppUseCase {
    fn ensure_passkey_management_enabled(&self) -> AppResult<()> {
        if self.webauthn.available().is_none() {
            return Err(AppError::Validation(
                "passkey authentication is not configured".into(),
            ));
        }

        Ok(())
    }

    fn ensure_passkey_authentication_enabled(&self, form_login_enabled: bool) -> AppResult<()> {
        if !form_login_enabled {
            return Err(AppError::Validation(
                "passkey authentication is unavailable while form login is disabled".into(),
            ));
        }

        self.ensure_passkey_management_enabled()
    }

    fn webauthn_runtime(&self) -> AppResult<&webauthn_rs::Webauthn> {
        self.webauthn
            .available()
            .map(Arc::as_ref)
            .ok_or_else(|| AppError::Validation("passkey authentication is not configured".into()))
    }

    async fn load_password_backed_user(&self, user_id: &str) -> AppResult<User> {
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

        if !user.account_kind.allows_local_credentials() || user.password_hash.is_none() {
            return Err(AppError::Validation(
                "passkeys require a password-backed account".into(),
            ));
        }

        Ok(user)
    }

    fn user_id_candidates_for_webauthn_uuid(user_uuid: Uuid) -> Vec<String> {
        let mut candidates = vec![user_uuid.to_string()];
        let compact = user_uuid.simple().to_string();
        if compact != candidates[0] {
            candidates.push(compact);
        }
        candidates
    }

    async fn load_password_backed_user_by_webauthn_uuid(&self, user_uuid: Uuid) -> AppResult<User> {
        for candidate in Self::user_id_candidates_for_webauthn_uuid(user_uuid) {
            match self.load_password_backed_user(&candidate).await {
                Ok(user) => return Ok(user),
                Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }

        Err(AppError::NotFound(format!("user {user_uuid}")))
    }

    async fn cleanup_expired_webauthn_challenges(&self) -> AppResult<()> {
        self.services
            .identity
            .webauthn
            .delete_expired_challenges(&Utc::now().to_rfc3339())
            .await?;
        Ok(())
    }

    fn parse_user_uuid(&self, user_id: &str) -> AppResult<Uuid> {
        Uuid::parse_str(user_id).map_err(|error| {
            AppError::Repository(format!("user id {user_id} is not a valid UUID: {error}"))
        })
    }

    fn trim_friendly_name(value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    }

    fn challenge_expired(record: &WebauthnChallengeRecord) -> bool {
        chrono::DateTime::parse_from_rfc3339(&record.expires_at)
            .map(|value| value.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(true)
    }

    fn encode_credential_id(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn deserialize_passkey(record: &WebauthnCredentialRecord) -> AppResult<Passkey> {
        serde_json::from_str(&record.credential_json).map_err(|error| {
            AppError::Repository(format!(
                "failed to decode stored passkey {}: {error}",
                record.id
            ))
        })
    }

    fn passkey_summary(record: WebauthnCredentialRecord) -> PasskeySummary {
        PasskeySummary {
            id: record.id,
            friendly_name: record.friendly_name,
            created_at: record.created_at,
            last_used_at: record.last_used_at,
        }
    }

    pub fn passkey_enabled(&self) -> bool {
        self.webauthn.available().is_some()
    }

    pub async fn webauthn_register_start(
        &self,
        actor: &User,
        _form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.ensure_passkey_management_enabled()?;
        self.cleanup_expired_webauthn_challenges().await?;

        let user = self.load_password_backed_user(&actor.id).await?;
        let existing_records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&user.id)
            .await?;
        let existing_passkeys = existing_records
            .iter()
            .map(Self::deserialize_passkey)
            .collect::<AppResult<Vec<_>>>()?;
        let exclude_credentials = (!existing_passkeys.is_empty()).then(|| {
            existing_passkeys
                .iter()
                .map(|passkey| passkey.cred_id().clone())
                .collect::<Vec<_>>()
        });

        let (options, state) = self
            .webauthn_runtime()?
            .start_passkey_registration(
                self.parse_user_uuid(&user.id)?,
                &user.username,
                &user.username,
                exclude_credentials,
            )
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey registration: {error}"))
            })?;

        let challenge = WebauthnChallengeRecord {
            id: Id::new().0,
            user_id: Some(user.id),
            challenge_type: WebauthnChallengeType::Registration,
            state_json: serde_json::to_string(&StoredChallengeState::Registration(state)).map_err(
                |error| {
                    AppError::Repository(format!(
                        "failed to persist passkey registration state: {error}"
                    ))
                },
            )?,
            created_at: Utc::now().to_rfc3339(),
            expires_at: (Utc::now() + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES))
                .to_rfc3339(),
        };

        self.services
            .identity
            .webauthn
            .create_challenge(challenge.clone())
            .await?;

        Ok(WebauthnChallengeStart {
            challenge_id: challenge.id,
            options_json: serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode passkey registration options: {error}"
                ))
            })?,
        })
    }

    pub async fn webauthn_register_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        response_json: &str,
        friendly_name: Option<String>,
        _form_login_enabled: bool,
    ) -> AppResult<PasskeySummary> {
        self.ensure_passkey_management_enabled()?;
        self.cleanup_expired_webauthn_challenges().await?;

        let user = self.load_password_backed_user(&actor.id).await?;
        let challenge = self
            .services
            .identity
            .webauthn
            .get_challenge(challenge_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("passkey challenge {challenge_id}")))?;

        if challenge.user_id.as_deref() != Some(user.id.as_str()) {
            return Err(AppError::Unauthorized(
                "passkey challenge does not belong to the current user".into(),
            ));
        }
        if challenge.challenge_type != WebauthnChallengeType::Registration {
            return Err(AppError::Validation(
                "passkey challenge is not a registration ceremony".into(),
            ));
        }
        if Self::challenge_expired(&challenge) {
            self.services
                .identity
                .webauthn
                .delete_challenge(challenge_id)
                .await?;
            return Err(AppError::Validation("passkey challenge has expired".into()));
        }

        self.services
            .identity
            .webauthn
            .delete_challenge(challenge_id)
            .await?;

        let registration = serde_json::from_str::<RegisterPublicKeyCredential>(response_json)
            .map_err(|error| {
                AppError::Validation(format!(
                    "invalid passkey registration response payload: {error}"
                ))
            })?;
        let state = match serde_json::from_str::<StoredChallengeState>(&challenge.state_json)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode stored passkey registration state: {error}"
                ))
            })? {
            StoredChallengeState::Registration(state) => state,
            StoredChallengeState::Authentication(_) => {
                return Err(AppError::Validation(
                    "passkey challenge stored an authentication ceremony".into(),
                ));
            }
        };

        let passkey = self
            .webauthn_runtime()?
            .finish_passkey_registration(&registration, &state)
            .map_err(|error| {
                AppError::Validation(format!("failed to finish passkey registration: {error}"))
            })?;
        let credential_id = Self::encode_credential_id(passkey.cred_id().as_ref());

        if self
            .services
            .identity
            .webauthn
            .get_credential_by_credential_id(&credential_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(
                "a passkey with this credential id is already registered".into(),
            ));
        }

        let created = self
            .services
            .identity
            .webauthn
            .create_credential(WebauthnCredentialRecord {
                id: Id::new().0,
                user_id: user.id,
                credential_id,
                credential_json: serde_json::to_string(&passkey).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist registered passkey credential: {error}"
                    ))
                })?,
                friendly_name: Self::trim_friendly_name(friendly_name),
                created_at: Utc::now().to_rfc3339(),
                last_used_at: None,
            })
            .await?;

        Ok(Self::passkey_summary(created))
    }

    pub async fn webauthn_authenticate_start(
        &self,
        username: Option<&str>,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;

        let (record, options_json) = if let Some(username) =
            username.map(str::trim).filter(|value| !value.is_empty())
        {
            let invalid_username_passkey =
                || AppError::Unauthorized("invalid passkey credentials".into());
            let user = self
                .services
                .identity
                .users
                .get_by_username(username)
                .await?
                .ok_or_else(invalid_username_passkey)?;

            if user.password_hash.is_none() {
                return Err(invalid_username_passkey());
            }

            let records = self
                .services
                .identity
                .webauthn
                .list_credentials_for_user(&user.id)
                .await?;
            let passkeys = records
                .iter()
                .map(Self::deserialize_passkey)
                .collect::<AppResult<Vec<_>>>()?;

            if passkeys.is_empty() {
                return Err(invalid_username_passkey());
            }

            let (options, state) = self
                .webauthn_runtime()?
                .start_passkey_authentication(&passkeys)
                .map_err(|error| {
                    AppError::Validation(format!("failed to start passkey authentication: {error}"))
                })?;

            let record = WebauthnChallengeRecord {
                id: Id::new().0,
                user_id: Some(user.id),
                challenge_type: WebauthnChallengeType::Authentication,
                state_json: serde_json::to_string(&StoredChallengeState::Authentication(
                    StoredAuthenticationState::Passkey(state),
                ))
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist passkey authentication state: {error}"
                    ))
                })?,
                created_at: Utc::now().to_rfc3339(),
                expires_at: (Utc::now() + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES))
                    .to_rfc3339(),
            };
            let options_json = serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode passkey authentication options: {error}"
                ))
            })?;
            (record, options_json)
        } else {
            let (options, state) = self
                .webauthn_runtime()?
                .start_discoverable_authentication()
                .map_err(|error| {
                    AppError::Validation(format!(
                        "failed to start discoverable passkey authentication: {error}"
                    ))
                })?;

            let record = WebauthnChallengeRecord {
                id: Id::new().0,
                user_id: None,
                challenge_type: WebauthnChallengeType::Authentication,
                state_json: serde_json::to_string(&StoredChallengeState::Authentication(
                    StoredAuthenticationState::Discoverable(state),
                ))
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist discoverable passkey authentication state: {error}"
                    ))
                })?,
                created_at: Utc::now().to_rfc3339(),
                expires_at: (Utc::now() + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES))
                    .to_rfc3339(),
            };
            let options_json = serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode discoverable passkey authentication options: {error}"
                ))
            })?;
            (record, options_json)
        };

        self.services
            .identity
            .webauthn
            .create_challenge(record.clone())
            .await?;

        Ok(WebauthnChallengeStart {
            challenge_id: record.id,
            options_json,
        })
    }

    pub async fn webauthn_authenticate_complete(
        &self,
        challenge_id: &str,
        response_json: &str,
        form_login_enabled: bool,
    ) -> AppResult<User> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;

        let challenge = self
            .services
            .identity
            .webauthn
            .get_challenge(challenge_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("passkey challenge {challenge_id}")))?;

        if challenge.challenge_type != WebauthnChallengeType::Authentication {
            return Err(AppError::Validation(
                "passkey challenge is not an authentication ceremony".into(),
            ));
        }
        if Self::challenge_expired(&challenge) {
            self.services
                .identity
                .webauthn
                .delete_challenge(challenge_id)
                .await?;
            return Err(AppError::Validation("passkey challenge has expired".into()));
        }

        self.services
            .identity
            .webauthn
            .delete_challenge(challenge_id)
            .await?;

        let credential =
            serde_json::from_str::<PublicKeyCredential>(response_json).map_err(|error| {
                AppError::Validation(format!(
                    "invalid passkey authentication response payload: {error}"
                ))
            })?;
        let state = match serde_json::from_str::<StoredChallengeState>(&challenge.state_json)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode stored passkey authentication state: {error}"
                ))
            })? {
            StoredChallengeState::Authentication(state) => state,
            StoredChallengeState::Registration(_) => {
                return Err(AppError::Validation(
                    "passkey challenge stored a registration ceremony".into(),
                ));
            }
        };

        match state {
            StoredAuthenticationState::Passkey(state) => {
                let user_id = challenge.user_id.as_deref().ok_or_else(|| {
                    AppError::Repository("authentication challenge missing user id".into())
                })?;
                let user = self.load_password_backed_user(user_id).await?;
                let records = self
                    .services
                    .identity
                    .webauthn
                    .list_credentials_for_user(&user.id)
                    .await?;
                let mut passkeys = records
                    .iter()
                    .map(Self::deserialize_passkey)
                    .collect::<AppResult<Vec<_>>>()?;
                let auth_result = self
                    .webauthn_runtime()?
                    .finish_passkey_authentication(&credential, &state)
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to finish passkey authentication: {error}"
                        ))
                    })?;
                let used_credential_id = Self::encode_credential_id(auth_result.cred_id());

                let updated_record = records
                    .into_iter()
                    .zip(passkeys.iter_mut())
                    .find_map(|(record, passkey)| {
                        (record.credential_id == used_credential_id).then_some((record, passkey))
                    })
                    .ok_or_else(|| {
                        AppError::Repository(format!(
                            "authenticated passkey credential {used_credential_id} was not found"
                        ))
                    })?;
                let (mut record, passkey) = updated_record;
                passkey.update_credential(&auth_result);
                record.credential_json = serde_json::to_string(passkey).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist updated passkey credential state: {error}"
                    ))
                })?;
                record.last_used_at = Some(Utc::now().to_rfc3339());
                self.services
                    .identity
                    .webauthn
                    .update_credential(record)
                    .await?;
                Ok(user)
            }
            StoredAuthenticationState::Discoverable(state) => {
                let (user_uuid, credential_id) = self
                    .webauthn_runtime()?
                    .identify_discoverable_authentication(&credential)
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to identify discoverable passkey authentication: {error}"
                        ))
                    })?;
                let credential_id = Self::encode_credential_id(credential_id);
                let user = self
                    .load_password_backed_user_by_webauthn_uuid(user_uuid)
                    .await?;
                let mut record = self
                    .services
                    .identity
                    .webauthn
                    .get_credential_by_credential_id(&credential_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized(
                            "discoverable passkey credential was not found".into(),
                        )
                    })?;
                if record.user_id != user.id {
                    return Err(AppError::Unauthorized(
                        "discoverable passkey credential does not belong to the resolved user"
                            .into(),
                    ));
                }
                let mut passkey = Self::deserialize_passkey(&record)?;
                let discoverable_key: DiscoverableKey = passkey.clone().into();
                let auth_result = self
                    .webauthn_runtime()?
                    .finish_discoverable_authentication(&credential, state, &[discoverable_key])
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to finish discoverable passkey authentication: {error}"
                        ))
                    })?;
                passkey.update_credential(&auth_result);
                record.credential_json = serde_json::to_string(&passkey).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist updated discoverable passkey state: {error}"
                    ))
                })?;
                record.last_used_at = Some(Utc::now().to_rfc3339());
                self.services
                    .identity
                    .webauthn
                    .update_credential(record)
                    .await?;
                Ok(user)
            }
        }
    }

    pub async fn list_my_passkeys(
        &self,
        actor: &User,
        _form_login_enabled: bool,
    ) -> AppResult<Vec<PasskeySummary>> {
        self.ensure_passkey_management_enabled()?;
        let records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&actor.id)
            .await?;
        Ok(records.into_iter().map(Self::passkey_summary).collect())
    }

    pub async fn delete_my_passkey(
        &self,
        actor: &User,
        credential_record_id: &str,
        _form_login_enabled: bool,
    ) -> AppResult<()> {
        self.ensure_passkey_management_enabled()?;
        self.services
            .identity
            .webauthn
            .delete_credential_for_user(credential_record_id, &actor.id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webauthn_uuid_lookup_candidates_include_legacy_compact_admin_id() {
        let user_uuid =
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");

        assert_eq!(
            AppUseCase::user_id_candidates_for_webauthn_uuid(user_uuid),
            vec![
                "00000000-0000-0000-0000-000000000001".to_string(),
                "00000000000000000000000000000001".to_string(),
            ],
        );
    }
}
