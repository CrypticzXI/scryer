use aws_lc_rs::digest;
use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use scryer_domain::{Id, User};
use url::Url;

use crate::{
    AppError, AppResult, AppUseCase, OAuthAuthorizationCodeRecord, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshTokenRecord,
};

pub const OAUTH_GENERIC_NATIVE_CLIENT_ID: &str = "generic-native";
pub const OAUTH_E2E_CLIENT_ID: &str = "e2e";
pub const OAUTH_E2E_CLIENT_ENV: &str = "SCRYER_ENABLE_E2E_OAUTH_CLIENT";
pub const OAUTH_LIBRARY_SCOPE: &str = "library";

const AUTHORIZATION_CODE_TTL_SECONDS: i64 = 5 * 60;
const OAUTH_SECRET_BYTES: usize = 32;
const CODE_PREFIX: &str = "scryer_oac";
const REFRESH_PREFIX: &str = "scryer_ort";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthClientInfo {
    pub client_id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthIssuedCode {
    pub code: String,
    pub record: OAuthAuthorizationCodeRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthTokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthConnectedAppSummary {
    pub grant_id: String,
    pub client_id: String,
    pub client_name: String,
    pub authorized_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AppUseCase {
    pub fn oauth_builtin_clients(&self) -> Vec<OAuthClientInfo> {
        let mut clients = vec![OAuthClientInfo {
            client_id: OAUTH_GENERIC_NATIVE_CLIENT_ID.to_string(),
            name: "Generic native integration".to_string(),
        }];
        if self.oauth_e2e_client_enabled() {
            clients.push(OAuthClientInfo {
                client_id: OAUTH_E2E_CLIENT_ID.to_string(),
                name: "Scryer E2E OAuth client".to_string(),
            });
        }
        clients
    }

    pub fn oauth_client_info(&self, client_id: &str) -> Option<OAuthClientInfo> {
        match client_id {
            OAUTH_GENERIC_NATIVE_CLIENT_ID => Some(OAuthClientInfo {
                client_id: client_id.to_string(),
                name: "Generic native integration".to_string(),
            }),
            OAUTH_E2E_CLIENT_ID if self.oauth_e2e_client_enabled() => Some(OAuthClientInfo {
                client_id: client_id.to_string(),
                name: "Scryer E2E OAuth client".to_string(),
            }),
            _ => None,
        }
    }

    pub fn validate_oauth_redirect_uri(
        &self,
        client_id: &str,
        redirect_uri: &str,
    ) -> AppResult<OAuthClientInfo> {
        let client = self
            .oauth_client_info(client_id)
            .ok_or_else(|| AppError::Validation("unknown OAuth client".into()))?;
        let url = Url::parse(redirect_uri)
            .map_err(|_| AppError::Validation("invalid redirect_uri".into()))?;
        match client_id {
            OAUTH_GENERIC_NATIVE_CLIENT_ID if is_loopback_redirect(&url) => Ok(client),
            OAUTH_E2E_CLIENT_ID if self.oauth_e2e_client_enabled() && is_e2e_redirect(&url) => {
                Ok(client)
            }
            _ => Err(AppError::Validation(
                "redirect_uri is not allowed for this OAuth client".into(),
            )),
        }
    }

    pub fn validate_oauth_pkce_request(
        &self,
        code_challenge: &str,
        code_challenge_method: &str,
    ) -> AppResult<()> {
        if code_challenge_method != "S256" {
            return Err(AppError::Validation(
                "code_challenge_method must be S256".into(),
            ));
        }
        validate_pkce_code_challenge(code_challenge)
    }

    pub fn validate_oauth_scope(&self, scope: Option<&str>) -> AppResult<String> {
        match scope.map(str::trim).filter(|scope| !scope.is_empty()) {
            None | Some(OAUTH_LIBRARY_SCOPE) => Ok(OAUTH_LIBRARY_SCOPE.to_string()),
            Some(_) => Err(AppError::Validation("invalid OAuth scope".into())),
        }
    }

    pub async fn create_oauth_authorization_code(
        &self,
        user: &User,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        code_challenge: &str,
        code_challenge_method: &str,
    ) -> AppResult<OAuthIssuedCode> {
        self.validate_oauth_redirect_uri(client_id, redirect_uri)?;
        let scope = self.validate_oauth_scope(Some(scope))?;
        self.validate_oauth_pkce_request(code_challenge, code_challenge_method)?;
        let id = Id::new().0;
        let secret = random_oauth_secret()?;
        let code = format!("{CODE_PREFIX}_{id}.{secret}");
        let now = Utc::now();
        let record = OAuthAuthorizationCodeRecord {
            id,
            code_hash: self.oauth_token_hash("authorization_code", &code),
            client_id: client_id.to_string(),
            user_id: user.id.clone(),
            redirect_uri: redirect_uri.to_string(),
            scope,
            code_challenge: code_challenge.to_string(),
            code_challenge_method: code_challenge_method.to_string(),
            created_at: now,
            expires_at: now + Duration::seconds(AUTHORIZATION_CODE_TTL_SECONDS),
            consumed_at: None,
        };
        let record = self
            .services
            .identity
            .oauth
            .create_authorization_code(record)
            .await?;
        Ok(OAuthIssuedCode { code, record })
    }

    pub async fn exchange_oauth_authorization_code(
        &self,
        client_id: &str,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> AppResult<OAuthTokenPair> {
        validate_pkce_code_verifier(code_verifier)?;
        let code_id = oauth_token_id(code, CODE_PREFIX)?;
        let Some(record) = self
            .services
            .identity
            .oauth
            .get_authorization_code(&code_id)
            .await?
        else {
            return Err(AppError::Unauthorized(
                "authorization code is invalid".into(),
            ));
        };
        if record.consumed_at.is_some() || record.expires_at <= Utc::now() {
            return Err(AppError::Unauthorized(
                "authorization code is expired or already used".into(),
            ));
        }
        if record.client_id != client_id || record.redirect_uri != redirect_uri {
            return Err(AppError::Unauthorized(
                "authorization code binding mismatch".into(),
            ));
        }
        let expected_hash = self.oauth_token_hash("authorization_code", code);
        if record.code_hash != expected_hash {
            return Err(AppError::Unauthorized(
                "authorization code is invalid".into(),
            ));
        }
        if record.code_challenge_method != "S256"
            || pkce_s256_challenge(code_verifier) != record.code_challenge
        {
            return Err(AppError::Unauthorized("PKCE verification failed".into()));
        }
        if !self
            .services
            .identity
            .oauth
            .consume_authorization_code(&record.id, Utc::now())
            .await?
        {
            return Err(AppError::Unauthorized(
                "authorization code is already used".into(),
            ));
        }
        let user = self
            .services
            .identity
            .users
            .get_by_id(&record.user_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth user no longer exists".into()))?;
        self.issue_oauth_token_pair(&user, client_id, &record.scope)
            .await
    }

    pub async fn refresh_oauth_token(
        &self,
        client_id: &str,
        refresh_token: &str,
    ) -> AppResult<OAuthTokenPair> {
        let token_id = oauth_token_id(refresh_token, REFRESH_PREFIX)?;
        let Some((token, grant)) = self
            .services
            .identity
            .oauth
            .get_refresh_token(&token_id)
            .await?
        else {
            return Err(AppError::Unauthorized("refresh token is invalid".into()));
        };
        if grant.client_id != client_id {
            return Err(AppError::Unauthorized(
                "refresh token client mismatch".into(),
            ));
        }
        if grant.revoked_at.is_some() || token.revoked_at.is_some() {
            return Err(AppError::Unauthorized("refresh token is revoked".into()));
        }
        let expected_hash = self.oauth_token_hash("refresh_token", refresh_token);
        if token.token_hash != expected_hash {
            return Err(AppError::Unauthorized("refresh token is invalid".into()));
        }
        if token.consumed_at.is_some() {
            self.services
                .identity
                .oauth
                .revoke_refresh_family(&grant.family_id, Utc::now(), "refresh_reuse")
                .await?;
            return Err(AppError::Unauthorized("refresh token was reused".into()));
        }
        let user = self
            .services
            .identity
            .users
            .get_by_id(&grant.user_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth user no longer exists".into()))?;
        let current_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?
            .unwrap_or_default();
        if current_session_version != grant.auth_session_version {
            self.services
                .identity
                .oauth
                .revoke_refresh_family(&grant.family_id, Utc::now(), "auth_session_changed")
                .await?;
            return Err(AppError::Unauthorized(
                "refresh token session is no longer valid".into(),
            ));
        }
        let (next_token, next_record) =
            self.new_refresh_token_record(&grant.id, &grant.family_id)?;
        let Some(rotation) = self
            .services
            .identity
            .oauth
            .rotate_refresh_token(&token.id, Utc::now(), next_record)
            .await?
        else {
            self.services
                .identity
                .oauth
                .revoke_refresh_family(&grant.family_id, Utc::now(), "refresh_reuse")
                .await?;
            return Err(AppError::Unauthorized("refresh token was reused".into()));
        };
        let access_token = self
            .issue_oauth_access_token(&user, &rotation.grant.client_id, &rotation.grant.id)
            .await?;
        Ok(OAuthTokenPair {
            access_token,
            refresh_token: next_token,
            expires_in: Self::OAUTH_ACCESS_TOKEN_TTL_SECONDS,
            scope: rotation.grant.scope,
        })
    }

    pub async fn revoke_oauth_refresh_token(&self, token: &str) -> AppResult<()> {
        let Ok(token_id) = oauth_token_id(token, REFRESH_PREFIX) else {
            return Ok(());
        };
        if let Some((_, grant)) = self
            .services
            .identity
            .oauth
            .get_refresh_token(&token_id)
            .await?
        {
            self.services
                .identity
                .oauth
                .revoke_refresh_family(&grant.family_id, Utc::now(), "revoked")
                .await?;
        }
        Ok(())
    }

    pub async fn touch_oauth_refresh_grant_last_used(
        &self,
        client_id: &str,
        grant_id: &str,
    ) -> AppResult<bool> {
        self.services
            .identity
            .oauth
            .touch_refresh_grant_last_used(grant_id, client_id, Utc::now())
            .await
    }

    pub async fn revoke_oauth_connected_app(
        &self,
        user_id: &str,
        grant_id: &str,
    ) -> AppResult<bool> {
        self.services
            .identity
            .oauth
            .revoke_refresh_grant(grant_id, user_id, Utc::now(), "user_revoked")
            .await
    }

    pub async fn list_oauth_connected_apps(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<OAuthConnectedAppSummary>> {
        let records = self
            .services
            .identity
            .oauth
            .list_connected_apps(user_id)
            .await?;
        Ok(records
            .into_iter()
            .filter_map(|record| connected_app_summary(self, record))
            .collect())
    }

    pub async fn revoke_oauth_refresh_grants_for_user(
        &self,
        user_id: &str,
        reason: &str,
    ) -> AppResult<u64> {
        self.services
            .identity
            .oauth
            .revoke_user_refresh_grants(user_id, Utc::now(), reason)
            .await
    }

    fn oauth_e2e_client_enabled(&self) -> bool {
        std::env::var(OAUTH_E2E_CLIENT_ENV)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    }

    fn oauth_token_hash(&self, context: &str, token: &str) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        crate::to_hex(hmac::sign(&key, format!("oauth:{context}:{token}").as_bytes()).as_ref())
    }

    async fn issue_oauth_token_pair(
        &self,
        user: &User,
        client_id: &str,
        scope: &str,
    ) -> AppResult<OAuthTokenPair> {
        let scope = self.validate_oauth_scope(Some(scope))?;
        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?
            .unwrap_or_default();
        let grant_id = Id::new().0;
        let family_id = Id::new().0;
        let now = Utc::now();
        let grant = OAuthRefreshGrantRecord {
            id: grant_id.clone(),
            family_id: family_id.clone(),
            user_id: user.id.clone(),
            client_id: client_id.to_string(),
            scope,
            auth_session_version,
            created_at: now,
            updated_at: now,
            last_used_at: None,
            revoked_at: None,
            revoked_reason: None,
        };
        let (refresh_token, token_record) = self.new_refresh_token_record(&grant_id, &family_id)?;
        let grant = self
            .services
            .identity
            .oauth
            .create_refresh_grant(grant, token_record)
            .await?;
        let access_token = self
            .issue_oauth_access_token(user, &grant.client_id, &grant.id)
            .await?;
        Ok(OAuthTokenPair {
            access_token,
            refresh_token,
            expires_in: Self::OAUTH_ACCESS_TOKEN_TTL_SECONDS,
            scope: grant.scope,
        })
    }

    fn new_refresh_token_record(
        &self,
        grant_id: &str,
        family_id: &str,
    ) -> AppResult<(String, OAuthRefreshTokenRecord)> {
        let id = Id::new().0;
        let secret = random_oauth_secret()?;
        let token = format!("{REFRESH_PREFIX}_{id}.{secret}");
        let record = OAuthRefreshTokenRecord {
            id,
            grant_id: grant_id.to_string(),
            family_id: family_id.to_string(),
            token_hash: self.oauth_token_hash("refresh_token", &token),
            created_at: Utc::now(),
            consumed_at: None,
            revoked_at: None,
        };
        Ok((token, record))
    }
}

fn connected_app_summary(
    app: &AppUseCase,
    record: OAuthConnectedAppRecord,
) -> Option<OAuthConnectedAppSummary> {
    let client = app.oauth_client_info(&record.client_id)?;
    Some(OAuthConnectedAppSummary {
        grant_id: record.grant_id,
        client_id: record.client_id,
        client_name: client.name,
        authorized_at: record.created_at,
        last_used_at: record.last_used_at,
    })
}

fn random_oauth_secret() -> AppResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = [0_u8; OAUTH_SECRET_BYTES];
    rng.fill(&mut bytes)
        .map_err(|_| AppError::Repository("failed to generate OAuth token".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn oauth_token_id(token: &str, prefix: &str) -> AppResult<String> {
    let Some((id_part, secret)) = token.split_once('.') else {
        return Err(AppError::Unauthorized("OAuth token is malformed".into()));
    };
    if secret.is_empty() {
        return Err(AppError::Unauthorized("OAuth token is malformed".into()));
    }
    id_part
        .strip_prefix(&format!("{prefix}_"))
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::Unauthorized("OAuth token is malformed".into()))
}

fn validate_pkce_code_challenge(code_challenge: &str) -> AppResult<()> {
    if code_challenge.len() != 43 || !code_challenge.bytes().all(is_base64url_byte) {
        return Err(AppError::Validation(
            "code_challenge must be a valid S256 base64url value".into(),
        ));
    }
    Ok(())
}

fn validate_pkce_code_verifier(code_verifier: &str) -> AppResult<()> {
    let len = code_verifier.len();
    if !(43..=128).contains(&len) || !code_verifier.bytes().all(is_pkce_verifier_byte) {
        return Err(AppError::Validation(
            "code_verifier must be 43-128 RFC7636 unreserved characters".into(),
        ));
    }
    Ok(())
}

fn is_base64url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn is_pkce_verifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn pkce_s256_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, code_verifier.as_bytes()).as_ref())
}

fn is_loopback_redirect(url: &Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
}

fn is_e2e_redirect(url: &Url) -> bool {
    url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "::1" | "[::1]" | "localhost")
        )
        && url.path().starts_with("/oauth/e2e/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_challenge_and_verifier_follow_rfc7636_syntax() {
        let verifier = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~abcdef";
        validate_pkce_code_verifier(verifier).expect("valid verifier");
        let challenge = pkce_s256_challenge(verifier);
        validate_pkce_code_challenge(&challenge).expect("valid S256 challenge");

        assert!(validate_pkce_code_verifier("too-short").is_err());
        assert!(validate_pkce_code_verifier(&"a".repeat(129)).is_err());
        assert!(
            validate_pkce_code_verifier(
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~abc+"
            )
            .is_err()
        );
        assert!(validate_pkce_code_challenge("plain").is_err());
        assert!(validate_pkce_code_challenge(&format!("{}=", challenge)).is_err());
    }

    #[test]
    fn native_loopback_redirects_follow_rfc8252_host_rules() {
        assert!(is_loopback_redirect(
            &Url::parse("http://127.0.0.1:49152/callback").expect("url")
        ));
        assert!(is_loopback_redirect(
            &Url::parse("http://[::1]:49152/callback").expect("url")
        ));

        assert!(!is_loopback_redirect(
            &Url::parse("http://localhost:49152/callback").expect("url")
        ));
        assert!(!is_loopback_redirect(
            &Url::parse("https://127.0.0.1:49152/callback").expect("url")
        ));
        assert!(!is_loopback_redirect(
            &Url::parse("scryer://oauth/callback").expect("url")
        ));
    }

    #[test]
    fn e2e_redirects_are_tightly_scoped_to_local_oauth_path() {
        assert!(is_e2e_redirect(
            &Url::parse("http://127.0.0.1:3000/oauth/e2e/callback").expect("url")
        ));
        assert!(is_e2e_redirect(
            &Url::parse("http://localhost:3000/oauth/e2e/callback").expect("url")
        ));

        assert!(!is_e2e_redirect(
            &Url::parse("http://127.0.0.1:3000/other").expect("url")
        ));
        assert!(!is_e2e_redirect(
            &Url::parse("https://localhost:3000/oauth/e2e/callback").expect("url")
        ));
    }
}
