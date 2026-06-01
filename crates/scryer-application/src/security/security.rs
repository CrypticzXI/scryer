use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use aws_lc_rs::hmac;

use super::*;
use crate::services::AppAssembly;
use crate::services::RuntimeFeature;
use crate::types::{
    AuthenticatedTokenClaims, BackupDownloadTicket, BackupDownloadTokenClaims,
    JwtLibraryPermissionClaim, JwtSessionScope, ReleaseCandidateTokenClaims,
};

impl AppUseCase {
    const BACKUP_DOWNLOAD_TOKEN_KIND: &'static str = "backup_download_v1";
    const BACKUP_DOWNLOAD_TOKEN_TTL_SECONDS: i64 = 5 * 60;
    const MFA_ENROLLMENT_TOKEN_TTL_SECONDS: i64 = 10 * 60;
    const RELEASE_CANDIDATE_TOKEN_KIND: &'static str = "release_candidate_v1";
    const RELEASE_CANDIDATE_TOKEN_TTL_SECONDS: i64 = 15 * 60;

    pub(crate) fn app_permission_claim_string(
        permission: scryer_domain::AppPermission,
    ) -> &'static str {
        match permission {
            scryer_domain::AppPermission::ManageUsers => "manageUsers",
            scryer_domain::AppPermission::ManagePermissions => "managePermissions",
            scryer_domain::AppPermission::ManageSystemSettings => "manageSystemSettings",
            scryer_domain::AppPermission::ManageCatalogSettings => "manageCatalogSettings",
        }
    }

    pub(crate) fn library_permission_claim_string(
        permission: scryer_domain::LibraryPermission,
    ) -> &'static str {
        match permission {
            scryer_domain::LibraryPermission::View => "view",
            scryer_domain::LibraryPermission::ManageTitles => "manageTitles",
            scryer_domain::LibraryPermission::ResolveImports => "resolveImports",
            scryer_domain::LibraryPermission::ManageLibrary => "manageLibrary",
            scryer_domain::LibraryPermission::Request => "request",
            scryer_domain::LibraryPermission::AutoApproveRequests => "autoApproveRequests",
        }
    }

    pub fn new(
        assembly: AppAssembly,
        auth: JwtAuthConfig,
        facet_registry: Arc<FacetRegistry>,
    ) -> Self {
        Self::new_with_webauthn(assembly, auth, facet_registry, None)
    }

    pub fn new_with_webauthn(
        assembly: AppAssembly,
        auth: JwtAuthConfig,
        facet_registry: Arc<FacetRegistry>,
        webauthn: Option<Arc<webauthn_rs::Webauthn>>,
    ) -> Self {
        Self {
            services: assembly.services,
            runtime: assembly.runtime,
            auth,
            facet_registry,
            pending_import_resolution_locks: Arc::new(std::sync::Mutex::new(HashSet::new())),
            jwt_signing_keys: Arc::new(RwLock::new(HashMap::new())),
            jwt_signing_keys_loaded: Arc::new(OnceCell::new()),
            jwt_signing_keys_seed_lock: Arc::new(Mutex::new(())),
            webauthn: webauthn.map(RuntimeFeature::enabled).unwrap_or_default(),
        }
    }

    pub(crate) fn hash_password(&self, password: &str) -> AppResult<String> {
        if password.trim().is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let phc_string = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|err| AppError::Repository(format!("password hashing failed: {err}")))?
            .to_string();
        Ok(format!("v2${phc_string}"))
    }

    pub(crate) async fn password_min_length(&self) -> AppResult<i32> {
        Ok(self
            .read_setting_i64_value(PASSWORD_MIN_LENGTH_KEY, None)
            .await?
            .unwrap_or(PASSWORD_MIN_LENGTH_MIN)
            .max(PASSWORD_MIN_LENGTH_MIN) as i32)
    }

    pub(crate) async fn validate_new_local_password(&self, password: &str) -> AppResult<()> {
        let min_length = self.password_min_length().await?;
        if password.chars().count() < min_length as usize {
            return Err(AppError::Validation(format!(
                "password must be at least {min_length} characters"
            )));
        }

        Ok(())
    }

    pub(crate) async fn default_admin_uses_bootstrap_password(&self) -> AppResult<bool> {
        let admin = self.find_or_create_default_user().await?;
        let Some(password_hash) = admin.password_hash.as_deref() else {
            return Ok(true);
        };

        self.validate_password("admin", password_hash)
    }

    pub(crate) fn validate_password(&self, password: &str, password_hash: &str) -> AppResult<bool> {
        if let Some(phc_string) = password_hash.strip_prefix("v2$") {
            let parsed = PasswordHash::new(phc_string)
                .map_err(|err| AppError::Validation(format!("invalid v2 password hash: {err}")))?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        } else if password_hash.starts_with("v1$") {
            self.validate_password_v1(password, password_hash)
        } else {
            Err(AppError::Validation(
                "unsupported password hash version".into(),
            ))
        }
    }

    fn validate_password_v1(&self, password: &str, password_hash: &str) -> AppResult<bool> {
        let mut parts = password_hash.splitn(3, '$');
        let _ = parts.next(); // "v1"

        let salt = parts
            .next()
            .ok_or_else(|| AppError::Validation("invalid password hash: missing salt".into()))?;
        let stored_hash = parts
            .next()
            .ok_or_else(|| AppError::Validation("invalid password hash: missing hash".into()))?;

        let candidate = sha256_hex(format!("{salt}{}", password));
        Ok(candidate == stored_hash)
    }

    fn canonical_app_permission_claims(user: &User) -> Vec<String> {
        let mut claims = user
            .authorization
            .app
            .to_permissions()
            .into_iter()
            .map(Self::app_permission_claim_string)
            .map(str::to_string)
            .collect::<Vec<_>>();
        claims.sort();
        claims.dedup();
        claims
    }

    fn canonical_library_permission_claims(user: &User) -> Vec<JwtLibraryPermissionClaim> {
        let mut claims = user
            .authorization
            .libraries
            .iter()
            .map(|(library_id, permissions)| {
                let mut permissions = permissions
                    .to_permissions()
                    .into_iter()
                    .map(Self::library_permission_claim_string)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                permissions.sort();
                permissions.dedup();
                JwtLibraryPermissionClaim {
                    library_id: library_id.clone(),
                    permissions,
                }
            })
            .collect::<Vec<_>>();
        claims.sort_by(|left, right| left.library_id.cmp(&right.library_id));
        claims
    }

    fn authorization_fingerprint(user: &User) -> String {
        let app_claims = Self::canonical_app_permission_claims(user).join("\n");
        let library_claims = Self::canonical_library_permission_claims(user)
            .into_iter()
            .map(|grant| format!("{}:{}", grant.library_id, grant.permissions.join(",")))
            .collect::<Vec<_>>()
            .join("\n");
        sha256_hex(format!("app\n{app_claims}\nlibrary\n{library_claims}"))
    }

    /// Derive a per-user JWT signing key:
    /// HMAC-SHA256(key=salt, msg="{password_hash}\n{authorization_fingerprint}").
    ///
    /// The salt is the registration secret baked into the binary, so an offline
    /// DB dump alone cannot forge tokens.
    pub(crate) fn derive_jwt_key(
        &self,
        password_hash: &str,
        authorization_fingerprint: &str,
    ) -> Vec<u8> {
        let signing_material = format!("{password_hash}\n{authorization_fingerprint}");
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        hmac::sign(&hmac_key, signing_material.as_bytes())
            .as_ref()
            .to_vec()
    }

    async fn user_with_authorization(&self, user: &User) -> AppResult<User> {
        if user.authorization.loaded {
            return Ok(user.clone());
        }
        let mut user = user.clone();
        user.authorization = self.load_user_authorization(&user).await?;
        Ok(user)
    }

    pub async fn load_user_for_auth_payload(&self, user: &User) -> AppResult<User> {
        let mut user = self
            .services
            .identity
            .users
            .get_by_id(&user.id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("token subject no longer exists".into()))?;
        user.authorization = self.load_user_authorization(&user).await?;
        Ok(user)
    }

    async fn derive_jwt_key_for_user(&self, user: &User) -> AppResult<Option<Vec<u8>>> {
        let user = self.user_with_authorization(user).await?;
        let signing_seed = user
            .password_hash
            .clone()
            .unwrap_or_else(|| format!("federated:{}", user.id));

        Ok(Some(self.derive_jwt_key(
            &signing_seed,
            &Self::authorization_fingerprint(&user),
        )))
    }

    async fn write_cached_jwt_signing_key(&self, user: &User, evict_first: bool) -> AppResult<()> {
        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        let mut cache = self.jwt_signing_keys.write().await;

        if evict_first {
            cache.remove(&user.id);
        }

        match self.derive_jwt_key_for_user(user).await? {
            Some(signing_key) => {
                cache.insert(user.id.clone(), signing_key);
            }
            None => {
                cache.remove(&user.id);
            }
        }

        Ok(())
    }

    pub(super) async fn cache_jwt_signing_key(&self, user: &User) -> AppResult<()> {
        self.write_cached_jwt_signing_key(user, false).await
    }

    pub(super) async fn refresh_cached_jwt_signing_key(&self, user: &User) -> AppResult<()> {
        self.write_cached_jwt_signing_key(user, true).await
    }

    pub(super) async fn evict_cached_jwt_signing_key(&self, user_id: &str) {
        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        self.jwt_signing_keys.write().await.remove(user_id);
    }

    pub(crate) async fn ensure_jwt_signing_keys_loaded(&self) -> AppResult<()> {
        if self.jwt_signing_keys_loaded.get().is_some() {
            return Ok(());
        }

        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        if self.jwt_signing_keys_loaded.get().is_some() {
            return Ok(());
        }

        let users = self.services.identity.users.list_all().await?;
        let mut cache = self.jwt_signing_keys.write().await;
        cache.clear();
        for user in users {
            if let Some(signing_key) = self.derive_jwt_key_for_user(&user).await? {
                cache.insert(user.id, signing_key);
            }
        }
        let _ = self.jwt_signing_keys_loaded.set(());
        Ok(())
    }

    pub fn token_lifetime(&self) -> i64 {
        self.auth.access_ttl_seconds as i64
    }

    pub fn mfa_enrollment_token_lifetime(&self) -> i64 {
        Self::MFA_ENROLLMENT_TOKEN_TTL_SECONDS
    }

    pub fn totp_step_up_verified_until(&self) -> chrono::DateTime<Utc> {
        Utc::now() + Duration::minutes(super::totp::TOTP_STEP_UP_TTL_MINUTES)
    }

    pub async fn issue_access_token(&self, actor: &User) -> AppResult<String> {
        self.issue_access_token_with_mfa(actor, None).await
    }

    pub async fn issue_access_token_with_mfa(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_and_scope(
            actor,
            mfa_verified_until,
            JwtSessionScope::Full,
            self.token_lifetime(),
        )
        .await
    }

    pub async fn issue_mfa_enrollment_token(&self, actor: &User) -> AppResult<String> {
        self.issue_access_token_with_mfa_and_scope(
            actor,
            None,
            JwtSessionScope::MfaEnrollment,
            self.mfa_enrollment_token_lifetime(),
        )
        .await
    }

    async fn issue_access_token_with_mfa_and_scope(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        auth_scope: JwtSessionScope,
        ttl_seconds: i64,
    ) -> AppResult<String> {
        let actor = self.load_user_for_auth_payload(actor).await?;
        let signing_seed = actor
            .password_hash
            .clone()
            .unwrap_or_else(|| format!("federated:{}", actor.id));

        let now = Utc::now();
        let iat = now.timestamp();
        let exp = (now + Duration::seconds(ttl_seconds)).timestamp();

        let app_permissions = Self::canonical_app_permission_claims(&actor);
        let library_permissions = Self::canonical_library_permission_claims(&actor);

        let claims = JwtClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            username: actor.username.clone(),
            app_permissions,
            library_permissions,
            mfa_verified_until: mfa_verified_until.map(|value| value.timestamp()),
            auth_scope,
        };

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key =
            self.derive_jwt_key(&signing_seed, &Self::authorization_fingerprint(&actor));
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);

        let token = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|err| AppError::Repository(format!("failed to issue token: {err}")))?;

        Ok(token)
    }

    fn derive_scoped_signing_key(&self, jwt_signing_key: &[u8], token_kind: &str) -> Vec<u8> {
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, jwt_signing_key);
        hmac::sign(&hmac_key, token_kind.as_bytes())
            .as_ref()
            .to_vec()
    }

    fn derive_backup_download_signing_key(&self, jwt_signing_key: &[u8]) -> Vec<u8> {
        self.derive_scoped_signing_key(jwt_signing_key, Self::BACKUP_DOWNLOAD_TOKEN_KIND)
    }

    fn derive_release_candidate_signing_key(&self, jwt_signing_key: &[u8]) -> Vec<u8> {
        self.derive_scoped_signing_key(jwt_signing_key, Self::RELEASE_CANDIDATE_TOKEN_KIND)
    }

    pub(crate) async fn backup_download_signing_key_for_actor(
        &self,
        actor: &User,
    ) -> AppResult<Vec<u8>> {
        self.ensure_jwt_signing_keys_loaded().await?;
        let cache = self.jwt_signing_keys.read().await;
        let jwt_signing_key = cache.get(&actor.id).cloned().ok_or_else(|| {
            AppError::Unauthorized(format!(
                "cannot resolve backup download signing key for actor {}",
                actor.id
            ))
        })?;

        Ok(self.derive_backup_download_signing_key(&jwt_signing_key))
    }

    pub(crate) async fn release_candidate_signing_key_for_actor(
        &self,
        actor: &User,
    ) -> AppResult<Vec<u8>> {
        self.ensure_jwt_signing_keys_loaded().await?;
        let cache = self.jwt_signing_keys.read().await;
        let jwt_signing_key = cache.get(&actor.id).cloned().ok_or_else(|| {
            AppError::Unauthorized(format!(
                "cannot resolve release candidate signing key for actor {}",
                actor.id
            ))
        })?;

        Ok(self.derive_release_candidate_signing_key(&jwt_signing_key))
    }

    fn submission_scope_claims(scope: &SubmissionScope) -> (&'static str, Option<String>) {
        match scope {
            SubmissionScope::Episode { episode_id } => ("episode", Some(episode_id.clone())),
            SubmissionScope::EpisodeSet { episode_ids } => (
                "episode_set",
                Some(serde_json::to_string(episode_ids).unwrap_or_else(|_| String::new())),
            ),
            SubmissionScope::Collection { collection_id } => {
                ("collection", Some(collection_id.clone()))
            }
            SubmissionScope::Title => ("title", None),
            SubmissionScope::Orphan => ("orphan", None),
        }
    }

    fn submission_scope_from_claims(
        scope_kind: &str,
        scope_id: Option<String>,
    ) -> AppResult<SubmissionScope> {
        match scope_kind {
            "episode" => Ok(SubmissionScope::Episode {
                episode_id: scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing episode scope id".into(),
                    )
                })?,
            }),
            "episode_set" => {
                let raw = scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing episode-set scope id".into(),
                    )
                })?;
                let mut episode_ids =
                    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_else(|_| {
                        raw.split(',')
                            .map(|value| value.trim().to_string())
                            .collect()
                    });
                episode_ids.retain(|episode_id| !episode_id.is_empty());
                episode_ids.sort();
                episode_ids.dedup();
                if episode_ids.is_empty() {
                    return Err(AppError::Unauthorized(
                        "release candidate token has empty episode-set scope".into(),
                    ));
                }
                Ok(SubmissionScope::EpisodeSet { episode_ids })
            }
            "collection" => Ok(SubmissionScope::Collection {
                collection_id: scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing collection scope id".into(),
                    )
                })?,
            }),
            "title" => Ok(SubmissionScope::Title),
            "orphan" => Ok(SubmissionScope::Orphan),
            _ => Err(AppError::Unauthorized(
                "release candidate token has unknown scope".into(),
            )),
        }
    }

    pub(crate) fn issue_release_candidate_token_with_signing_key(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        selection: &QueuedReleaseSelection,
        signing_key: &[u8],
    ) -> AppResult<String> {
        let source_hint = selection
            .source_hint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Validation("release candidate token requires a source hint".into())
            })?;
        let source_title = selection
            .source_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Validation("release candidate token requires a source title".into())
            })?;

        let now = Utc::now();
        let iat = now.timestamp();
        let exp = (now + Duration::seconds(Self::RELEASE_CANDIDATE_TOKEN_TTL_SECONDS)).timestamp();
        let (scope_kind, scope_id) = Self::submission_scope_claims(scope);
        let claims = ReleaseCandidateTokenClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            kind: Self::RELEASE_CANDIDATE_TOKEN_KIND.to_string(),
            title_id: title_id.to_string(),
            scope_kind: scope_kind.to_string(),
            scope_id,
            source_hint: source_hint.to_string(),
            source_kind: selection.source_kind,
            source_title: source_title.to_string(),
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = jsonwebtoken::EncodingKey::from_secret(signing_key);

        jsonwebtoken::encode(&header, &claims, &key).map_err(|err| {
            AppError::Repository(format!("failed to issue release candidate token: {err}"))
        })
    }

    pub(crate) fn issue_backup_download_token_with_signing_key(
        &self,
        actor: &User,
        filename: &str,
        signing_key: &[u8],
    ) -> AppResult<BackupDownloadTicket> {
        let now = Utc::now();
        let iat = now.timestamp();
        let expires_at = now + Duration::seconds(Self::BACKUP_DOWNLOAD_TOKEN_TTL_SECONDS);
        let claims = BackupDownloadTokenClaims {
            sub: actor.id.clone(),
            exp: expires_at.timestamp(),
            iat,
            iss: self.auth.issuer.clone(),
            kind: Self::BACKUP_DOWNLOAD_TOKEN_KIND.to_string(),
            filename: filename.to_string(),
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = jsonwebtoken::EncodingKey::from_secret(signing_key);
        let token = jsonwebtoken::encode(&header, &claims, &key).map_err(|err| {
            AppError::Repository(format!("failed to issue backup download token: {err}"))
        })?;

        Ok(BackupDownloadTicket {
            token,
            expires_at: expires_at.to_rfc3339(),
        })
    }

    pub async fn issue_backup_download_token(
        &self,
        actor: &User,
        filename: &str,
    ) -> AppResult<BackupDownloadTicket> {
        let signing_key = self.backup_download_signing_key_for_actor(actor).await?;
        self.issue_backup_download_token_with_signing_key(actor, filename, &signing_key)
    }

    pub async fn issue_release_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        selection: &QueuedReleaseSelection,
    ) -> AppResult<String> {
        let signing_key = self.release_candidate_signing_key_for_actor(actor).await?;
        self.issue_release_candidate_token_with_signing_key(
            actor,
            title_id,
            scope,
            selection,
            &signing_key,
        )
    }

    pub async fn verify_release_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        token: &str,
    ) -> AppResult<QueuedReleaseSelection> {
        let (selection, claimed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, token)
            .await?;
        if &claimed_scope != scope {
            return Err(AppError::Unauthorized(
                "release candidate token scope does not match request".into(),
            ));
        }
        Ok(selection)
    }

    fn backup_download_token_subject(&self, token: &str) -> AppResult<String> {
        let unverified = jsonwebtoken::dangerous::insecure_decode::<BackupDownloadTokenClaims>(
            token,
        )
        .map_err(|err| AppError::Unauthorized(format!("malformed backup download token: {err}")))?;
        let subject = unverified.claims.sub.trim();
        if subject.is_empty() {
            return Err(AppError::Unauthorized(
                "backup download token subject is empty".into(),
            ));
        }
        Ok(subject.to_string())
    }

    pub async fn verify_backup_download_token(
        &self,
        actor: &User,
        filename: &str,
        token: &str,
    ) -> AppResult<()> {
        let signing_key = self.backup_download_signing_key_for_actor(actor).await?;
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);
        let claims = jsonwebtoken::decode::<BackupDownloadTokenClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid backup download token: {err}")))?
            .claims;

        if claims.kind != Self::BACKUP_DOWNLOAD_TOKEN_KIND {
            return Err(AppError::Unauthorized(
                "invalid backup download token kind".into(),
            ));
        }
        if claims.sub != actor.id {
            return Err(AppError::Unauthorized(
                "backup download token subject does not match actor".into(),
            ));
        }
        if claims.filename != filename {
            return Err(AppError::Unauthorized(
                "backup download token filename does not match request".into(),
            ));
        }

        Ok(())
    }

    pub async fn authorize_backup_download_ticket(
        &self,
        filename: &str,
        token: &str,
    ) -> AppResult<User> {
        let subject = self.backup_download_token_subject(token)?;
        let actor = self
            .services
            .identity
            .users
            .get_by_id(&subject)
            .await?
            .ok_or_else(|| AppError::Unauthorized("unknown backup download subject".into()))?;
        let actor = self.attach_user_authorization(actor).await?;
        self.require_app_permission(&actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.verify_backup_download_token(&actor, filename, token)
            .await?;
        Ok(actor)
    }

    pub async fn verify_release_candidate_token_for_signed_scope(
        &self,
        actor: &User,
        title_id: &str,
        token: &str,
    ) -> AppResult<(QueuedReleaseSelection, SubmissionScope)> {
        let signing_key = self.release_candidate_signing_key_for_actor(actor).await?;
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);
        let claims = jsonwebtoken::decode::<ReleaseCandidateTokenClaims>(token, &key, &validation)
            .map_err(|err| {
                AppError::Unauthorized(format!("invalid release candidate token: {err}"))
            })?
            .claims;

        if claims.kind != Self::RELEASE_CANDIDATE_TOKEN_KIND {
            return Err(AppError::Unauthorized(
                "invalid release candidate token kind".into(),
            ));
        }
        if claims.sub != actor.id {
            return Err(AppError::Unauthorized(
                "release candidate token subject does not match actor".into(),
            ));
        }
        if claims.title_id != title_id {
            return Err(AppError::Unauthorized(
                "release candidate token title does not match request".into(),
            ));
        }

        let claimed_scope =
            Self::submission_scope_from_claims(&claims.scope_kind, claims.scope_id)?;

        Ok((
            QueuedReleaseSelection {
                source_hint: Some(claims.source_hint),
                source_kind: claims.source_kind,
                source_title: Some(claims.source_title),
            },
            claimed_scope,
        ))
    }

    pub async fn authenticate_token(&self, token: &str) -> AppResult<User> {
        self.authenticate_token_with_claims(token)
            .await
            .map(|(user, _)| user)
    }

    pub async fn authenticate_token_with_claims(
        &self,
        token: &str,
    ) -> AppResult<(User, AuthenticatedTokenClaims)> {
        // Decode claims without signature verification to extract the subject (user ID).
        let unverified = jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(token)
            .map_err(|err| AppError::Unauthorized(format!("malformed token: {err}")))?;

        let user_id = &unverified.claims.sub;
        self.ensure_jwt_signing_keys_loaded().await?;

        // Now verify the signature with the per-user key.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);

        let signing_key = self
            .jwt_signing_keys
            .read()
            .await
            .get(user_id)
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("unknown token subject".into()))?;
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);

        let verified = jsonwebtoken::decode::<JwtClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid token: {err}")))?;
        let claims = verified.claims;
        self.services
            .identity
            .users
            .get_by_id(&claims.sub)
            .await?
            .map(|mut user| {
                user.password_hash = None;
                (
                    user,
                    AuthenticatedTokenClaims {
                        mfa_verified_until: claims.mfa_verified_until,
                        session_scope: claims.auth_scope,
                    },
                )
            })
            .ok_or_else(|| AppError::Unauthorized("token subject no longer exists".into()))
    }

    pub async fn authenticate_credentials(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<User> {
        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::Validation("username is required".into()));
        }
        let password = password.trim();
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let user = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {username} not found")))?;

        let password_hash = user
            .password_hash
            .as_ref()
            .ok_or_else(|| AppError::Unauthorized("credentials unavailable".into()))?;

        if !self.validate_password(password, password_hash)? {
            return Err(AppError::Unauthorized("invalid credentials".into()));
        }

        // Online migration: re-hash v1 passwords with Argon2id on successful login.
        // Must return the updated user so the caller's JWT signing key matches the DB.
        if password_hash.starts_with("v1$")
            && let Ok(new_hash) = self.hash_password(password)
        {
            match self
                .services
                .identity
                .users
                .update_password_hash(&user.id, new_hash)
                .await
            {
                Ok(updated) => {
                    self.cache_jwt_signing_key(&updated).await?;
                    tracing::info!(user_id = %user.id, "migrated password hash from v1 to v2");
                    return Ok(updated);
                }
                Err(err) => {
                    tracing::warn!(user_id = %user.id, error = %err, "failed to migrate password hash from v1 to v2");
                }
            }
        }

        self.cache_jwt_signing_key(&user).await?;
        Ok(user)
    }
}
