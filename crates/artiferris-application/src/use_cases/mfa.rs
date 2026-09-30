use std::sync::Arc;

use chrono::Utc;
use artiferris_domain::audit::SecurityAuditRecord;
use artiferris_domain::email::EmailPort;
use artiferris_domain::error::DomainError;
use artiferris_domain::mfa::{BackupCodePort, TotpCredentialPort};
use artiferris_domain::user::{PasswordHasherPort, User, UserRepositoryPort, UserSecurityPort};
use rand::Rng;
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use uuid::Uuid;

use crate::error::ApplicationError;

const ISSUER: &str = "ArtiFerris";
const BACKUP_CODE_COUNT: usize = 10;

fn generate_secret_base32() -> String {
    let mut bytes = [0u8; 20];
    rand::rng().fill_bytes(&mut bytes);
    Secret::new_stack(bytes).to_base32()
}

/// SHA1/6 digits/30s step, 1 step of skew: the defaults nearly every authenticator app assumes.
fn build_totp(secret_base32: &str, username: &str) -> Result<Totp, ApplicationError> {
    let secret = Secret::try_from_base32(secret_base32).map_err(|_| ApplicationError::Domain(DomainError::Infrastructure("stored TOTP secret is not valid base32".to_string())))?;
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret)
        .with_account_name(username)
        .with_issuer(Some(ISSUER))
        .build()
        .map_err(|e| ApplicationError::Domain(DomainError::Infrastructure(e.to_string())))
}

fn generate_backup_code() -> String {
    let mut bytes = [0u8; 16]; // 128 bits
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Stored as `<salt>:<sha256>`, salted so equal codes never share a stored value. `pub` so tests can seed a known code.
pub fn hash_backup_code(plaintext: &str) -> String {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt_hex = hex::encode(salt);
    let digest = Sha256::digest(format!("{salt_hex}{plaintext}").as_bytes());
    format!("{salt_hex}:{}", hex::encode(digest))
}

/// Recomputes the hash with the stored salt. A value without `:` is a legacy unsalted hash and is compared the old way,
/// or codes issued before salting would be unusable.
pub fn verify_backup_code(plaintext: &str, stored: &str) -> bool {
    let Some((salt_hex, expected_digest_hex)) = stored.split_once(':') else {
        return hex::encode(Sha256::digest(plaintext.as_bytes())) == stored;
    };
    let digest = Sha256::digest(format!("{salt_hex}{plaintext}").as_bytes());
    hex::encode(digest) == expected_digest_hex
}

/// Test-support helper: computes a currently-valid code for a base32 secret.
pub fn generate_current_totp_code(secret_base32: &str) -> String {
    build_totp(secret_base32, "").expect("a base32 secret produced by EnrollTotpUseCase is always valid").generate_current().to_string()
}

/// Test-support helper: a second, still-unused code following one already consumed.
pub fn generate_totp_code_after_step(secret_base32: &str, after_step: i64) -> String {
    let totp = build_totp(secret_base32, "").expect("a base32 secret produced by EnrollTotpUseCase is always valid");
    totp.generate((after_step as u64 + 1) * 30).to_string()
}

fn generate_backup_codes() -> (Vec<String>, Vec<String>) {
    let plaintext: Vec<String> = (0..BACKUP_CODE_COUNT).map(|_| generate_backup_code()).collect();
    let hashes = plaintext.iter().map(|c| hash_backup_code(c)).collect();
    (plaintext, hashes)
}

pub(crate) async fn verify_current_password(users: &dyn UserRepositoryPort, hasher: &dyn PasswordHasherPort, user_id: Uuid, current_password: &str) -> Result<(), ApplicationError> {
    let user = users.find_by_id(user_id).await?.ok_or(ApplicationError::InvalidCredentials)?;
    if !hasher.verify(current_password, &user.password_hash).await? {
        return Err(ApplicationError::InvalidCredentials);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfaStatus {
    pub totp_enabled: bool,
    pub backup_codes_remaining: i64,
    pub passkey_count: i64,
}

pub struct GetMfaStatusUseCase {
    totp: Arc<dyn TotpCredentialPort>,
    backup_codes: Arc<dyn BackupCodePort>,
    passkeys: Arc<dyn artiferris_domain::webauthn::WebauthnCredentialPort>,
}

impl GetMfaStatusUseCase {
    pub fn new(totp: Arc<dyn TotpCredentialPort>, backup_codes: Arc<dyn BackupCodePort>, passkeys: Arc<dyn artiferris_domain::webauthn::WebauthnCredentialPort>) -> Self {
        Self { totp, backup_codes, passkeys }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<MfaStatus, ApplicationError> {
        let totp_enabled = self.totp.get(user_id).await?.is_some_and(|c| c.confirmed);
        let backup_codes_remaining = if totp_enabled { self.backup_codes.count_unused(user_id).await? } else { 0 };
        let passkey_count = self.passkeys.count_for_user(user_id).await?;
        Ok(MfaStatus { totp_enabled, backup_codes_remaining, passkey_count })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpEnrollment {
    pub secret_base32: String,
    pub otpauth_url: String,
}

pub struct EnrollTotpUseCase {
    totp: Arc<dyn TotpCredentialPort>,
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasherPort>,
}

impl EnrollTotpUseCase {
    pub fn new(totp: Arc<dyn TotpCredentialPort>, users: Arc<dyn UserRepositoryPort>, hasher: Arc<dyn PasswordHasherPort>) -> Self {
        Self { totp, users, hasher }
    }

    /// Refused once confirmed; replaces an unconfirmed attempt. `current_password` is `Some` for `/api/me/mfa/*`: a
    /// hijacked session token alone must not plant a second factor. It is `None` for the first-time setup flow, gated
    /// by `mfa_token`, which already proves the password.
    pub async fn execute(&self, user_id: Uuid, username: &str, current_password: Option<&str>) -> Result<TotpEnrollment, ApplicationError> {
        if let Some(current_password) = current_password {
            verify_current_password(self.users.as_ref(), self.hasher.as_ref(), user_id, current_password).await?;
        }

        let secret_base32 = generate_secret_base32();
        let totp = build_totp(&secret_base32, username)?;
        let otpauth_url = totp.to_url().map_err(|e| ApplicationError::Domain(DomainError::Infrastructure(e.to_string())))?;
        if !self.totp.begin_enrollment(user_id, &secret_base32, Utc::now()).await? {
            return Err(ApplicationError::MfaAlreadyEnabled);
        }
        Ok(TotpEnrollment { secret_base32, otpauth_url })
    }
}

/// The address an account notice may go to: one the user's organization has verified as theirs. A self-registered address is only a claim.
pub(crate) async fn verified_address(security: &dyn UserSecurityPort, user: &User) -> Option<String> {
    let email = user.email.as_deref()?;
    match security.find_by_verified_email(user.organization_id, email).await {
        Ok(Some(holder)) if holder.id == user.id => Some(email.to_string()),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("could not check whether an address is verified, sending nothing: {e}");
            None
        }
    }
}

/// An enrolment left unconfirmed for longer than this can't be confirmed any more.
const TOTP_ENROLLMENT_TTL_MINUTES: i64 = 15;

pub struct ConfirmTotpUseCase {
    totp: Arc<dyn TotpCredentialPort>,
    backup_codes: Arc<dyn BackupCodePort>,
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    email: Arc<dyn EmailPort>,
}

impl ConfirmTotpUseCase {
    pub fn new(totp: Arc<dyn TotpCredentialPort>, backup_codes: Arc<dyn BackupCodePort>, users: Arc<dyn UserRepositoryPort>, security: Arc<dyn UserSecurityPort>, email: Arc<dyn EmailPort>) -> Self {
        Self { totp, backup_codes, users, security, email }
    }

    /// Returns the plaintext backup codes — the only time they are ever visible; only their hashes are persisted.
    pub async fn execute(&self, user_id: Uuid, username: &str, code: &str) -> Result<Vec<String>, ApplicationError> {
        let credential = self.totp.get(user_id).await?.ok_or(ApplicationError::MfaNotEnrolled)?;
        if credential.confirmed {
            return Err(ApplicationError::MfaAlreadyEnabled);
        }
        if Utc::now() - credential.created_at > chrono::Duration::minutes(TOTP_ENROLLMENT_TTL_MINUTES) {
            return Err(ApplicationError::MfaEnrollmentExpired);
        }
        let totp = build_totp(&credential.secret, username)?;
        let step = totp.check_current(code).ok_or(ApplicationError::InvalidMfaCode)?;

        if !self.totp.confirm(user_id, credential.created_at, step as i64).await? {
            return Err(ApplicationError::InvalidMfaCode);
        }
        let (plaintext, hashes) = generate_backup_codes();
        self.backup_codes.replace_all(user_id, &hashes, None).await?;

        // Best-effort: enrollment already succeeded, a delivery failure must not undo it.
        if let Ok(Some(user)) = self.users.find_by_id(user_id).await {
            if let Some(email) = verified_address(self.security.as_ref(), &user).await {
                let language = self.email.language_for(user.id).await;
                let content = crate::email_templates::mfa_enrolled(language, username, crate::email_templates::EnrolledMethod::AuthenticatorApp);
                if let Err(e) = self.email.send(user.organization_id, &email, &content.subject, &content.text, &content.html).await {
                    tracing::warn!("failed to send MFA-enrollment confirmation email to {email}: {e}");
                }
            }
        }
        Ok(plaintext)
    }
}

pub struct VerifyTotpUseCase {
    totp: Arc<dyn TotpCredentialPort>,
}

impl VerifyTotpUseCase {
    pub fn new(totp: Arc<dyn TotpCredentialPort>) -> Self {
        Self { totp }
    }

    /// No username: `account_name` only feeds `to_url()`, unused here, so an empty value is fine.
    pub async fn execute(&self, user_id: Uuid, code: &str) -> Result<(), ApplicationError> {
        let credential = self.totp.get(user_id).await?.filter(|c| c.confirmed).ok_or(ApplicationError::MfaNotEnrolled)?;
        let totp = build_totp(&credential.secret, "")?;
        let step = totp.check_current(code).ok_or(ApplicationError::InvalidMfaCode)?;
        if credential.last_used_step.is_some_and(|last| step as i64 <= last) {
            return Err(ApplicationError::InvalidMfaCode);
        }
        if !self.totp.set_last_used_step(user_id, step as i64).await? {
            return Err(ApplicationError::InvalidMfaCode);
        }
        Ok(())
    }
}

pub struct VerifyBackupCodeUseCase {
    backup_codes: Arc<dyn BackupCodePort>,
}

impl VerifyBackupCodeUseCase {
    pub fn new(backup_codes: Arc<dyn BackupCodePort>) -> Self {
        Self { backup_codes }
    }

    pub async fn execute(&self, user_id: Uuid, code: &str) -> Result<(), ApplicationError> {
        if self.backup_codes.try_consume(user_id, code).await? {
            Ok(())
        } else {
            Err(ApplicationError::InvalidMfaCode)
        }
    }
}

pub struct DisableTotpUseCase {
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    totp: Arc<dyn TotpCredentialPort>,
    backup_codes: Arc<dyn BackupCodePort>,
}

impl DisableTotpUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, hasher: Arc<dyn PasswordHasherPort>, totp: Arc<dyn TotpCredentialPort>, backup_codes: Arc<dyn BackupCodePort>) -> Self {
        Self { users, hasher, totp, backup_codes }
    }

    pub async fn execute(&self, user_id: Uuid, current_password: &str) -> Result<(), ApplicationError> {
        verify_current_password(self.users.as_ref(), self.hasher.as_ref(), user_id, current_password).await?;
        self.totp.delete(user_id).await?;
        self.backup_codes.delete_all(user_id).await?;
        Ok(())
    }
}

pub struct RegenerateBackupCodesUseCase {
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    totp: Arc<dyn TotpCredentialPort>,
    backup_codes: Arc<dyn BackupCodePort>,
}

impl RegenerateBackupCodesUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, hasher: Arc<dyn PasswordHasherPort>, totp: Arc<dyn TotpCredentialPort>, backup_codes: Arc<dyn BackupCodePort>) -> Self {
        Self { users, hasher, totp, backup_codes }
    }

    /// `audit` is written in the same transaction as the new set.
    pub async fn execute(&self, user_id: Uuid, current_password: &str, audit: Option<&SecurityAuditRecord>) -> Result<Vec<String>, ApplicationError> {
        verify_current_password(self.users.as_ref(), self.hasher.as_ref(), user_id, current_password).await?;
        if !self.totp.get(user_id).await?.is_some_and(|c| c.confirmed) {
            return Err(ApplicationError::MfaNotEnrolled);
        }
        let (plaintext, hashes) = generate_backup_codes();
        self.backup_codes.replace_all(user_id, &hashes, audit).await?;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use artiferris_domain::mfa::TotpCredential;
    use artiferris_domain::user::{User, Username};
    use artiferris_domain::webauthn::{WebauthnCredential, WebauthnCredentialPort};

    use super::*;
    use crate::use_cases::verification_test_support::FakeVerification;

    /// Always empty: passkey coverage lives in `use_cases::webauthn`'s own tests.
    struct FakeWebauthnCredentials;

    #[async_trait]
    impl WebauthnCredentialPort for FakeWebauthnCredentials {
        async fn list_for_user(&self, _user_id: Uuid) -> Result<Vec<WebauthnCredential>, DomainError> {
            Ok(vec![])
        }
        async fn insert(&self, _credential: &WebauthnCredential, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            Ok(())
        }
        async fn update_passkey_data(&self, _id: Uuid, _passkey_data: Vec<u8>) -> Result<(), DomainError> {
            Ok(())
        }
        async fn delete(&self, _id: Uuid, _user_id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            Ok(())
        }
        async fn count_for_user(&self, _user_id: Uuid) -> Result<i64, DomainError> {
            Ok(0)
        }
    }

    struct FakeTotp {
        credentials: Mutex<HashMap<Uuid, TotpCredential>>,
    }

    impl FakeTotp {
        fn new() -> Self {
            Self { credentials: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl TotpCredentialPort for FakeTotp {
        async fn get(&self, user_id: Uuid) -> Result<Option<TotpCredential>, DomainError> {
            Ok(self.credentials.lock().unwrap().get(&user_id).cloned())
        }
        async fn begin_enrollment(&self, user_id: Uuid, secret: &str, created_at: chrono::DateTime<Utc>) -> Result<bool, DomainError> {
            let mut credentials = self.credentials.lock().unwrap();
            if credentials.get(&user_id).is_some_and(|c| c.confirmed) {
                return Ok(false);
            }
            credentials.insert(user_id, TotpCredential { user_id, secret: secret.to_string(), confirmed: false, last_used_step: None, created_at });
            Ok(true)
        }
        async fn confirm(&self, user_id: Uuid, enrollment_created_at: chrono::DateTime<Utc>, step: i64) -> Result<bool, DomainError> {
            let mut credentials = self.credentials.lock().unwrap();
            match credentials.get_mut(&user_id) {
                Some(c) if !c.confirmed && c.created_at == enrollment_created_at => {
                    c.confirmed = true;
                    c.last_used_step = Some(step);
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
        async fn set_last_used_step(&self, user_id: Uuid, step: i64) -> Result<bool, DomainError> {
            let mut credentials = self.credentials.lock().unwrap();
            let Some(c) = credentials.get_mut(&user_id) else { return Ok(false) };
            if c.last_used_step.is_some_and(|last| last >= step) {
                return Ok(false);
            }
            c.last_used_step = Some(step);
            Ok(true)
        }
        async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
            self.credentials.lock().unwrap().remove(&user_id);
            Ok(())
        }
    }

    struct FakeBackupCodes {
        by_user: Mutex<HashMap<Uuid, HashMap<String, bool>>>,
    }

    impl FakeBackupCodes {
        fn new() -> Self {
            Self { by_user: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl BackupCodePort for FakeBackupCodes {
        async fn replace_all(&self, user_id: Uuid, code_hashes: &[String], _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().insert(user_id, code_hashes.iter().map(|h| (h.clone(), false)).collect());
            Ok(())
        }
        async fn try_consume(&self, user_id: Uuid, plaintext_code: &str) -> Result<bool, DomainError> {
            let mut by_user = self.by_user.lock().unwrap();
            let Some(codes) = by_user.get_mut(&user_id) else { return Ok(false) };
            let Some(matching_hash) = codes.iter().find(|(hash, used)| !**used && verify_backup_code(plaintext_code, hash)).map(|(hash, _)| hash.clone()) else {
                return Ok(false);
            };
            *codes.get_mut(&matching_hash).unwrap() = true;
            Ok(true)
        }
        async fn count_unused(&self, user_id: Uuid) -> Result<i64, DomainError> {
            Ok(self.by_user.lock().unwrap().get(&user_id).map(|codes| codes.values().filter(|used| !**used).count()).unwrap_or(0) as i64)
        }
        async fn delete_all(&self, user_id: Uuid) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().remove(&user_id);
            Ok(())
        }
    }

    struct FakeUsers {
        users: Mutex<HashMap<Uuid, User>>,
    }

    impl FakeUsers {
        fn new() -> Self {
            Self { users: Mutex::new(HashMap::new()) }
        }

        fn with_user(user: User) -> Self {
            let mut users = HashMap::new();
            users.insert(user.id, user);
            Self { users: Mutex::new(users) }
        }
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUsers {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError> {
            Ok(self.users.lock().unwrap().get(&id).cloned())
        }
        async fn find_by_username(&self, username: &Username) -> Result<Option<User>, DomainError> {
            Ok(self.users.lock().unwrap().values().find(|u| &u.username == username).cloned())
        }
        async fn find_by_email(&self, email: &str) -> Result<Option<User>, DomainError> {
            Ok(self.users.lock().unwrap().values().find(|u| u.email.as_deref() == Some(email)).cloned())
        }
        async fn list_all(&self) -> Result<Vec<User>, DomainError> {
            Ok(self.users.lock().unwrap().values().cloned().collect())
        }
        async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<User>, DomainError> {
            Ok(self.users.lock().unwrap().values().filter(|u| ids.contains(&u.id)).cloned().collect())
        }
        async fn count_by_organization(&self, organization_id: Uuid) -> Result<i64, DomainError> {
            Ok(self.users.lock().unwrap().values().filter(|u| u.organization_id == organization_id).count() as i64)
        }
        async fn search_by_organization(&self, organization_id: Uuid, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
            let query = query.to_lowercase();
            let mut matches: Vec<User> = self
                .users
                .lock()
                .unwrap()
                .values()
                .filter(|u| u.organization_id == organization_id && u.username.as_str().to_lowercase().contains(&query))
                .cloned()
                .collect();
            matches.sort_by(|a, b| a.username.as_str().cmp(b.username.as_str()));
            matches.truncate(limit as usize);
            Ok(matches)
        }
        async fn search_all_organizations(&self, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
            let query = query.to_lowercase();
            let mut matches: Vec<User> =
                self.users.lock().unwrap().values().filter(|u| u.username.as_str().to_lowercase().contains(&query)).cloned().collect();
            matches.sort_by(|a, b| a.username.as_str().cmp(b.username.as_str()));
            matches.truncate(limit as usize);
            Ok(matches)
        }
        async fn insert(&self, user: &User) -> Result<(), DomainError> {
            self.users.lock().unwrap().insert(user.id, user.clone());
            Ok(())
        }
        async fn delete(&self, id: Uuid) -> Result<(), DomainError> {
            self.users.lock().unwrap().remove(&id);
            Ok(())
        }
        async fn update_password(&self, id: Uuid, new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> {
            if let Some(u) = self.users.lock().unwrap().get_mut(&id) {
                u.password_hash = new_password_hash;
            }
            Ok(())
        }
        async fn set_super_admin(&self, _id: Uuid, _is_super_admin: bool) -> Result<(), DomainError> {
            Ok(())
        }
        async fn set_organization_admin(&self, _id: Uuid, _is_organization_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            Ok(())
        }
        async fn delete_unless_last_super_admin(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            self.users.lock().unwrap().remove(&id);
            Ok(true)
        }
        async fn set_super_admin_unless_last(&self, _id: Uuid, _is_super_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            Ok(true)
        }
    }

    struct FakeHasher;

    #[async_trait]
    impl PasswordHasherPort for FakeHasher {
        async fn hash(&self, plain_password: &str) -> Result<String, DomainError> {
            Ok(format!("hashed:{plain_password}"))
        }
        async fn verify(&self, plain_password: &str, hash: &str) -> Result<bool, DomainError> {
            Ok(hash == format!("hashed:{plain_password}"))
        }
    }

    struct FakeEmail {
        sent: Mutex<Vec<(Uuid, String, String, String, String)>>,
    }

    impl FakeEmail {
        fn new() -> Self {
            Self { sent: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl EmailPort for FakeEmail {
        async fn send(&self, organization_id: Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError> {
            self.sent.lock().unwrap().push((organization_id, to.to_string(), subject.to_string(), text_body.to_string(), html_body.to_string()));
            Ok(())
        }
    }

    fn sample_user() -> User {
        User { id: Uuid::new_v4(), username: Username::parse("florian").unwrap(), password_hash: "hashed:s3cret!".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }
    }

    fn code_for(secret_base32: &str, username: &str) -> String {
        build_totp(secret_base32, username).unwrap().generate_current().to_string()
    }

    #[tokio::test]
    async fn enrolling_returns_a_secret_and_an_otpauth_url() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher));

        let enrollment = use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap();

        assert!(!enrollment.secret_base32.is_empty());
        assert!(enrollment.otpauth_url.starts_with("otpauth://totp/"));
    }

    /// A hijacked session token alone must not enroll a TOTP credential: the current password is required.
    #[tokio::test]
    async fn enrolling_totp_requires_the_current_password() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher));

        let err = use_case.execute(user.id, "florian", Some("wrong-password")).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidCredentials));
        assert!(totp_port.get(user.id).await.unwrap().is_none(), "a failed enroll attempt must not create a credential");

        let enrollment = use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        assert!(!enrollment.secret_base32.is_empty());
    }

    /// The first-time setup flow passes `None` and skips the password check.
    #[tokio::test]
    async fn enrolling_totp_with_no_password_supplied_skips_the_check() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher));

        let enrollment = use_case.execute(user.id, "florian", None).await.unwrap();
        assert!(!enrollment.secret_base32.is_empty());
    }

    #[tokio::test]
    async fn confirming_with_the_right_code_activates_totp_and_returns_backup_codes() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        let codes = ConfirmTotpUseCase::new(totp_port.clone(), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", &code).await.unwrap();

        assert_eq!(codes.len(), 10);
        assert!(totp_port.get(user.id).await.unwrap().unwrap().confirmed);
        assert_eq!(backup_codes.count_unused(user.id).await.unwrap(), 10);
    }

    #[tokio::test]
    async fn two_parallel_confirms_with_the_right_code_let_exactly_one_through_and_make_one_set_of_backup_codes() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        let confirm = Arc::new(ConfirmTotpUseCase::new(totp_port, backup_codes, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())));

        let attempts: Vec<_> = (0..8)
            .map(|_| {
                let confirm = confirm.clone();
                let code = code.clone();
                let user_id = user.id;
                tokio::spawn(async move { confirm.execute(user_id, "florian", &code).await })
            })
            .collect();
        let results: Vec<_> = futures::future::join_all(attempts).await.into_iter().map(|r| r.unwrap()).collect();

        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    }

    /// The confirm reads the credential, then a new enrolment replaces it before the confirm writes.
    #[tokio::test]
    async fn a_confirm_racing_a_new_enrolment_does_not_confirm_the_new_secret() {
        struct EnrollsBeforeConfirm(Arc<FakeTotp>);
        #[async_trait]
        impl TotpCredentialPort for EnrollsBeforeConfirm {
            async fn get(&self, user_id: Uuid) -> Result<Option<TotpCredential>, DomainError> {
                self.0.get(user_id).await
            }
            async fn begin_enrollment(&self, user_id: Uuid, secret: &str, created_at: chrono::DateTime<Utc>) -> Result<bool, DomainError> {
                self.0.begin_enrollment(user_id, secret, created_at).await
            }
            async fn confirm(&self, user_id: Uuid, enrollment_created_at: chrono::DateTime<Utc>, step: i64) -> Result<bool, DomainError> {
                self.0.begin_enrollment(user_id, &generate_secret_base32(), Utc::now()).await?;
                self.0.confirm(user_id, enrollment_created_at, step).await
            }
            async fn set_last_used_step(&self, user_id: Uuid, step: i64) -> Result<bool, DomainError> {
                self.0.set_last_used_step(user_id, step).await
            }
            async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
                self.0.delete(user_id).await
            }
        }

        let inner = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enrollment = EnrollTotpUseCase::new(inner.clone(), users, Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        let err = ConfirmTotpUseCase::new(Arc::new(EnrollsBeforeConfirm(inner.clone())), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", &code).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidMfaCode));
        assert!(!inner.get(user.id).await.unwrap().unwrap().confirmed);
        assert_eq!(backup_codes.count_unused(user.id).await.unwrap(), 0, "no backup codes for an enrolment that was not confirmed");
    }

    #[tokio::test]
    async fn enrolling_over_a_confirmed_credential_is_refused_and_leaves_it_alone() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enroll = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher));
        let enrollment = enroll.execute(user.id, "florian", None).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        ConfirmTotpUseCase::new(totp_port.clone(), backup_codes, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", &code).await.unwrap();

        let err = enroll.execute(user.id, "florian", None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::MfaAlreadyEnabled));
        let stored = totp_port.get(user.id).await.unwrap().unwrap();
        assert!(stored.confirmed);
        assert_eq!(stored.secret, enrollment.secret_base32);
    }

    #[tokio::test]
    async fn confirming_sends_a_notification_email_when_the_user_has_a_verified_one_on_file() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let mut user = sample_user();
        user.email = Some("florian@example.com".to_string());
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let email = Arc::new(FakeEmail::new());
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        ConfirmTotpUseCase::new(totp_port, backup_codes, users, Arc::new(FakeVerification::of(&[&user])), email.clone()).execute(user.id, "florian", &code).await.unwrap();

        let sent = email.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "florian@example.com");
        assert!(sent[0].3.contains("TOTP"));
    }

    #[tokio::test]
    async fn confirming_sends_nothing_to_an_address_nobody_verified() {
        let totp_port = Arc::new(FakeTotp::new());
        let mut user = sample_user();
        user.email = Some("someone-else@example.com".to_string());
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let email = Arc::new(FakeEmail::new());
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        ConfirmTotpUseCase::new(totp_port, Arc::new(FakeBackupCodes::new()), users, Arc::new(FakeVerification::nobody()), email.clone()).execute(user.id, "florian", &code).await.unwrap();

        assert!(email.sent.lock().unwrap().is_empty(), "a self-registered address is a claim, not a mailbox this account owns");
    }

    #[tokio::test]
    async fn confirming_sends_nothing_when_the_address_is_verified_for_a_different_account() {
        let totp_port = Arc::new(FakeTotp::new());
        let mut user = sample_user();
        user.email = Some("shared@example.com".to_string());
        let mut holder = sample_user();
        holder.organization_id = user.organization_id;
        holder.email = Some("shared@example.com".to_string());
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let email = Arc::new(FakeEmail::new());
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        ConfirmTotpUseCase::new(totp_port, Arc::new(FakeBackupCodes::new()), users, Arc::new(FakeVerification::of(&[&holder])), email.clone()).execute(user.id, "florian", &code).await.unwrap();

        assert!(email.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_enrollment_left_unconfirmed_for_too_long_can_no_longer_be_confirmed() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let secret = generate_secret_base32();
        totp_port.begin_enrollment(user.id, &secret, Utc::now() - chrono::Duration::minutes(TOTP_ENROLLMENT_TTL_MINUTES + 1)).await.unwrap();
        let code = code_for(&secret, "florian");

        let err = ConfirmTotpUseCase::new(totp_port.clone(), Arc::new(FakeBackupCodes::new()), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()))
            .execute(user.id, "florian", &code)
            .await
            .unwrap_err();

        assert!(matches!(err, ApplicationError::MfaEnrollmentExpired));
        assert!(!totp_port.get(user.id).await.unwrap().unwrap().confirmed);
    }

    #[tokio::test]
    async fn an_enrollment_inside_its_window_is_still_confirmed() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let secret = generate_secret_base32();
        totp_port.begin_enrollment(user.id, &secret, Utc::now() - chrono::Duration::minutes(TOTP_ENROLLMENT_TTL_MINUTES - 1)).await.unwrap();
        let code = code_for(&secret, "florian");

        ConfirmTotpUseCase::new(totp_port.clone(), Arc::new(FakeBackupCodes::new()), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()))
            .execute(user.id, "florian", &code)
            .await
            .unwrap();

        assert!(totp_port.get(user.id).await.unwrap().unwrap().confirmed);
    }

    #[tokio::test]
    async fn confirming_with_the_wrong_code_fails() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();

        let err = ConfirmTotpUseCase::new(totp_port, backup_codes, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", "000000").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    /// Confirming an already confirmed credential must not regenerate (and invalidate) the backup codes.
    #[tokio::test]
    async fn confirming_again_after_already_confirmed_is_refused_and_does_not_touch_backup_codes() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        let original_codes =
            ConfirmTotpUseCase::new(totp_port.clone(), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user_id, "florian", &code).await.unwrap();

        let err = ConfirmTotpUseCase::new(totp_port, backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user_id, "florian", &code).await.unwrap_err();

        assert!(matches!(err, ApplicationError::MfaAlreadyEnabled));
        let verify = VerifyBackupCodeUseCase::new(backup_codes);
        assert!(verify.execute(user_id, &original_codes[0]).await.is_ok(), "the original backup codes must still be valid after a refused re-confirm");
    }

    #[tokio::test]
    async fn re_enrolling_after_confirmation_is_refused() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        ConfirmTotpUseCase::new(totp_port.clone(), backup_codes, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user_id, "florian", &code).await.unwrap();

        let err = EnrollTotpUseCase::new(totp_port, users, Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap_err();
        assert!(matches!(err, ApplicationError::MfaAlreadyEnabled));
    }

    #[tokio::test]
    async fn verify_totp_accepts_a_correct_code_once_and_then_rejects_a_replay() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        ConfirmTotpUseCase::new(totp_port.clone(), backup_codes, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user_id, "florian", &code).await.unwrap();

        let verify = VerifyTotpUseCase::new(totp_port);
        let err = verify.execute(user_id, &code).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn verify_totp_rejects_the_code_when_the_atomic_advance_loses_the_race() {
        struct AlwaysLosesTheRace(Arc<FakeTotp>);
        #[async_trait]
        impl TotpCredentialPort for AlwaysLosesTheRace {
            async fn get(&self, user_id: Uuid) -> Result<Option<TotpCredential>, DomainError> {
                self.0.get(user_id).await
            }
            async fn begin_enrollment(&self, user_id: Uuid, secret: &str, created_at: chrono::DateTime<Utc>) -> Result<bool, DomainError> {
                self.0.begin_enrollment(user_id, secret, created_at).await
            }
            async fn confirm(&self, user_id: Uuid, enrollment_created_at: chrono::DateTime<Utc>, step: i64) -> Result<bool, DomainError> {
                self.0.confirm(user_id, enrollment_created_at, step).await
            }
            async fn set_last_used_step(&self, _user_id: Uuid, _step: i64) -> Result<bool, DomainError> {
                Ok(false)
            }
            async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
                self.0.delete(user_id).await
            }
        }

        let inner = Arc::new(FakeTotp::new());
        let user_id = Uuid::new_v4();
        let secret_base32 = generate_secret_base32();
        let created_at = Utc::now();
        inner.begin_enrollment(user_id, &secret_base32, created_at).await.unwrap();
        inner.confirm(user_id, created_at, 0).await.unwrap();
        let code = code_for(&secret_base32, "florian");

        let verify = VerifyTotpUseCase::new(Arc::new(AlwaysLosesTheRace(inner)));
        let err = verify.execute(user_id, &code).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn verify_totp_fails_when_not_enrolled() {
        let totp_port = Arc::new(FakeTotp::new());
        let verify = VerifyTotpUseCase::new(totp_port);
        let err = verify.execute(Uuid::new_v4(), "123456").await.unwrap_err();
        assert!(matches!(err, ApplicationError::MfaNotEnrolled));
    }

    #[tokio::test]
    async fn verify_totp_fails_while_still_unconfirmed() {
        let totp_port = Arc::new(FakeTotp::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");

        let err = VerifyTotpUseCase::new(totp_port).execute(user_id, &code).await.unwrap_err();
        assert!(matches!(err, ApplicationError::MfaNotEnrolled));
    }

    #[tokio::test]
    async fn verify_backup_code_accepts_a_valid_code_exactly_once() {
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user_id = Uuid::new_v4();
        backup_codes.replace_all(user_id, &[hash_backup_code("abc123")], None).await.unwrap();

        let verify = VerifyBackupCodeUseCase::new(backup_codes);
        verify.execute(user_id, "abc123").await.unwrap();
        let err = verify.execute(user_id, "abc123").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn disabling_totp_requires_the_current_password() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        ConfirmTotpUseCase::new(totp_port.clone(), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", &code).await.unwrap();

        let disable = DisableTotpUseCase::new(users, Arc::new(FakeHasher), totp_port.clone(), backup_codes.clone());
        let err = disable.execute(user.id, "wrong-password").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidCredentials));
        assert!(totp_port.get(user.id).await.unwrap().is_some(), "a failed disable must not remove the credential");

        disable.execute(user.id, "s3cret!").await.unwrap();
        assert!(totp_port.get(user.id).await.unwrap().is_none());
        assert_eq!(backup_codes.count_unused(user.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn regenerating_backup_codes_invalidates_the_old_set() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users.clone(), Arc::new(FakeHasher)).execute(user.id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        let original_codes = ConfirmTotpUseCase::new(totp_port.clone(), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user.id, "florian", &code).await.unwrap();

        let regenerate = RegenerateBackupCodesUseCase::new(users, Arc::new(FakeHasher), totp_port, backup_codes.clone());
        let new_codes = regenerate.execute(user.id, "s3cret!", None).await.unwrap();

        assert_ne!(original_codes, new_codes);
        let verify = VerifyBackupCodeUseCase::new(backup_codes);
        assert!(verify.execute(user.id, &original_codes[0]).await.is_err(), "an old backup code must no longer work");
        assert!(verify.execute(user.id, &new_codes[0]).await.is_ok());
    }

    #[tokio::test]
    async fn regenerating_backup_codes_without_totp_enrolled_fails() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));

        let regenerate = RegenerateBackupCodesUseCase::new(users, Arc::new(FakeHasher), totp_port, backup_codes);
        let err = regenerate.execute(user.id, "s3cret!", None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::MfaNotEnrolled));
    }

    #[tokio::test]
    async fn mfa_status_reports_disabled_when_never_enrolled() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let status = GetMfaStatusUseCase::new(totp_port, backup_codes, Arc::new(FakeWebauthnCredentials)).execute(Uuid::new_v4()).await.unwrap();
        assert_eq!(status, MfaStatus { totp_enabled: false, backup_codes_remaining: 0, passkey_count: 0 });
    }

    #[test]
    fn generated_backup_codes_have_128_bits_of_entropy() {
        let code = generate_backup_code();
        assert_eq!(hex::decode(&code).unwrap().len(), 16, "a backup code must decode to 16 bytes (128 bits), not 5 (40 bits)");
    }

    #[test]
    fn two_backup_codes_with_the_same_plaintext_hash_differently_due_to_salt() {
        let hash_a = hash_backup_code("abc123");
        let hash_b = hash_backup_code("abc123");
        assert_ne!(hash_a, hash_b, "identical plaintext must still produce different stored hashes across calls, via a random salt");
    }

    #[test]
    fn a_salted_hash_still_verifies_correctly() {
        let hash = hash_backup_code("abc123");
        assert!(verify_backup_code("abc123", &hash));
        assert!(!verify_backup_code("wrong-code", &hash));
    }

    /// Backup codes stored as a bare sha256 hex (no `<salt>:`) must still verify.
    #[test]
    fn verify_backup_code_falls_back_to_the_legacy_unsalted_comparison_for_a_colon_less_stored_value() {
        let legacy_hash = hex::encode(Sha256::digest(b"abc123"));
        assert!(!legacy_hash.contains(':'), "sanity check: a legacy hash has no salt separator");

        assert!(verify_backup_code("abc123", &legacy_hash), "a still-unused legacy backup code must keep verifying after salting was introduced");
        assert!(!verify_backup_code("wrong-code", &legacy_hash));
    }

    /// A user holding an unused legacy code can log in with it.
    #[tokio::test]
    async fn a_legacy_pre_fix_backup_code_still_verifies_and_consumes_through_the_use_case() {
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user_id = Uuid::new_v4();
        let legacy_hash = hex::encode(Sha256::digest(b"legacy-code"));
        backup_codes.replace_all(user_id, &[legacy_hash], None).await.unwrap();

        let verify = VerifyBackupCodeUseCase::new(backup_codes.clone());
        verify.execute(user_id, "legacy-code").await.expect("a still-unused legacy backup code must keep working after salting was introduced");

        let err = verify.execute(user_id, "legacy-code").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn mfa_status_reports_enabled_with_remaining_backup_codes_after_confirmation() {
        let totp_port = Arc::new(FakeTotp::new());
        let backup_codes = Arc::new(FakeBackupCodes::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let enrollment = EnrollTotpUseCase::new(totp_port.clone(), users, Arc::new(FakeHasher)).execute(user_id, "florian", Some("s3cret!")).await.unwrap();
        let code = code_for(&enrollment.secret_base32, "florian");
        ConfirmTotpUseCase::new(totp_port.clone(), backup_codes.clone(), Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new())).execute(user_id, "florian", &code).await.unwrap();

        let status = GetMfaStatusUseCase::new(totp_port, backup_codes, Arc::new(FakeWebauthnCredentials)).execute(user_id).await.unwrap();
        assert_eq!(status, MfaStatus { totp_enabled: true, backup_codes_remaining: 10, passkey_count: 0 });
    }
}
