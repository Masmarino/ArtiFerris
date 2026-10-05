use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use artiferris_domain::audit::{SecurityAuditRecord, SecurityEvent};
use artiferris_domain::email::EmailPort;
use artiferris_domain::error::DomainError;
use artiferris_domain::mfa::{BackupCodePort, TotpCredentialPort};
use artiferris_domain::user::{PasswordHasherPort, UserRepositoryPort, UserSecurityPort};
use artiferris_domain::webauthn::{WebauthnCredential, WebauthnCredentialPort};
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::error::ApplicationError;
use crate::use_cases::mfa::{has_any_factor, verify_current_password};

const CEREMONY_TTL_MINUTES: i64 = 5;
/// Past either limit the oldest ceremony is dropped, so unauthenticated starts can't grow the store without bound.
const MAX_CEREMONIES: usize = 10_000;
const MAX_CEREMONIES_PER_USER: usize = 5;
const MAX_PASSKEYS_PER_USER: usize = 20;
const MAX_PASSKEY_NAME_CHARS: usize = 64;

fn ensure_room_for_another_passkey(registered: usize) -> Result<(), ApplicationError> {
    if registered >= MAX_PASSKEYS_PER_USER {
        return Err(ApplicationError::Domain(DomainError::Validation(format!("an account can hold at most {MAX_PASSKEYS_PER_USER} passkeys"))));
    }
    Ok(())
}

enum CeremonyState {
    Registration(PasskeyRegistration),
    Authentication(PasskeyAuthentication),
}

struct CeremonyEntry {
    user_id: Uuid,
    state: CeremonyState,
    expires_at: DateTime<Utc>,
    seq: u64,
}

#[derive(Default)]
struct Ceremonies {
    entries: HashMap<Uuid, CeremonyEntry>,
    /// Oldest first. Every entry lives the same time, so this is also the order they expire in.
    by_age: BTreeMap<u64, Uuid>,
    per_user: HashMap<Uuid, Vec<u64>>,
    next_seq: u64,
}

impl Ceremonies {
    fn remove(&mut self, challenge_id: Uuid) -> Option<CeremonyEntry> {
        let entry = self.entries.remove(&challenge_id)?;
        self.by_age.remove(&entry.seq);
        if let Some(seqs) = self.per_user.get_mut(&entry.user_id) {
            seqs.retain(|seq| *seq != entry.seq);
            if seqs.is_empty() {
                self.per_user.remove(&entry.user_id);
            }
        }
        Some(entry)
    }

    /// Only looks at the oldest entries, so the cost follows what actually expired.
    fn drop_expired(&mut self, now: DateTime<Utc>) {
        while let Some(oldest) = self.by_age.values().next().copied() {
            if self.entries[&oldest].expires_at > now {
                break;
            }
            self.remove(oldest);
        }
    }

    fn drop_oldest(&mut self) {
        if let Some(oldest) = self.by_age.values().next().copied() {
            self.remove(oldest);
        }
    }

    fn drop_oldest_of(&mut self, user_id: Uuid) {
        let oldest = self.per_user.get(&user_id).and_then(|seqs| seqs.first()).map(|seq| self.by_age[seq]);
        if let Some(oldest) = oldest {
            self.remove(oldest);
        }
    }
}

/// Holds in-progress WebAuthn ceremonies between `start` and `finish`. Deliberately in-process, not persisted — a restart mid-ceremony just makes the client retry.
pub struct PasskeyCeremonyStore {
    ceremonies: Mutex<Ceremonies>,
    ttl: Duration,
    max_total: usize,
    max_per_user: usize,
}

impl Default for PasskeyCeremonyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyCeremonyStore {
    pub fn new() -> Self {
        Self::with_limits(Duration::minutes(CEREMONY_TTL_MINUTES), MAX_CEREMONIES, MAX_CEREMONIES_PER_USER)
    }

    fn with_limits(ttl: Duration, max_total: usize, max_per_user: usize) -> Self {
        Self { ceremonies: Mutex::new(Ceremonies::default()), ttl, max_total, max_per_user }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ceremonies> {
        self.ceremonies.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn insert(&self, user_id: Uuid, state: CeremonyState) -> Uuid {
        let mut ceremonies = self.lock();
        let now = Utc::now();
        ceremonies.drop_expired(now);
        while ceremonies.per_user.get(&user_id).is_some_and(|seqs| seqs.len() >= self.max_per_user) {
            ceremonies.drop_oldest_of(user_id);
        }
        while ceremonies.entries.len() >= self.max_total {
            ceremonies.drop_oldest();
        }
        let challenge_id = Uuid::new_v4();
        let seq = ceremonies.next_seq;
        ceremonies.next_seq += 1;
        ceremonies.entries.insert(challenge_id, CeremonyEntry { user_id, state, expires_at: now + self.ttl, seq });
        ceremonies.by_age.insert(seq, challenge_id);
        ceremonies.per_user.entry(user_id).or_default().push(seq);
        challenge_id
    }

    /// Single use, like a nonce — removes the entry, not just reads it.
    fn take(&self, challenge_id: Uuid, expected_user_id: Uuid) -> Option<CeremonyEntry> {
        let mut ceremonies = self.lock();
        ceremonies.drop_expired(Utc::now());
        let entry = ceremonies.remove(challenge_id)?;
        if entry.user_id != expected_user_id {
            return None;
        }
        Some(entry)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().entries.len()
    }
}

fn deserialize_passkey(credential: &WebauthnCredential) -> Result<Passkey, ApplicationError> {
    serde_json::from_slice(&credential.passkey_data).map_err(|e| ApplicationError::Domain(artiferris_domain::error::DomainError::Infrastructure(format!("corrupted stored passkey: {e}"))))
}

fn serialize_passkey(passkey: &Passkey) -> Vec<u8> {
    serde_json::to_vec(passkey).expect("Passkey serialization is infallible")
}

/// `Option`, not a bare `Webauthn`: an invalid `PUBLIC_URL` makes the client fail to construct, and passkeys degrade to unavailable rather than crashing server startup.
fn require_webauthn(webauthn: &Option<Webauthn>) -> Result<&Webauthn, ApplicationError> {
    webauthn.as_ref().ok_or(ApplicationError::PasskeysUnavailable)
}

pub struct StartPasskeyRegistrationUseCase {
    webauthn: Arc<Option<Webauthn>>,
    credentials: Arc<dyn WebauthnCredentialPort>,
    ceremonies: Arc<PasskeyCeremonyStore>,
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasherPort>,
}

impl StartPasskeyRegistrationUseCase {
    pub fn new(
        webauthn: Arc<Option<Webauthn>>,
        credentials: Arc<dyn WebauthnCredentialPort>,
        ceremonies: Arc<PasskeyCeremonyStore>,
        users: Arc<dyn UserRepositoryPort>,
        hasher: Arc<dyn PasswordHasherPort>,
    ) -> Self {
        Self { webauthn, credentials, ceremonies, users, hasher }
    }

    /// `current_password` is `Some` for `/api/me/mfa/*`, as for `EnrollTotpUseCase`: a hijacked session token alone
    /// must not enroll a passkey. It is `None` for the first-time setup flow, gated by `mfa_token`.
    pub async fn execute(&self, user_id: Uuid, username: &str, current_password: Option<&str>) -> Result<(Uuid, CreationChallengeResponse), ApplicationError> {
        if let Some(current_password) = current_password {
            verify_current_password(self.users.as_ref(), self.hasher.as_ref(), user_id, current_password).await?;
        }
        let webauthn = require_webauthn(&self.webauthn)?;
        let existing = self.credentials.list_for_user(user_id).await?;
        ensure_room_for_another_passkey(existing.len())?;
        let exclude: Vec<CredentialID> = existing.iter().map(deserialize_passkey).collect::<Result<Vec<_>, _>>()?.iter().map(|pk| pk.cred_id().clone()).collect();

        let (ccr, registration) = webauthn
            .start_passkey_registration(user_id, username, username, if exclude.is_empty() { None } else { Some(exclude) })
            .map_err(|e| ApplicationError::Domain(artiferris_domain::error::DomainError::Infrastructure(e.to_string())))?;

        let challenge_id = self.ceremonies.insert(user_id, CeremonyState::Registration(registration));
        Ok((challenge_id, ccr))
    }
}

pub struct FinishPasskeyRegistrationUseCase {
    webauthn: Arc<Option<Webauthn>>,
    credentials: Arc<dyn WebauthnCredentialPort>,
    ceremonies: Arc<PasskeyCeremonyStore>,
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    email: Arc<dyn EmailPort>,
}

impl FinishPasskeyRegistrationUseCase {
    pub fn new(
        webauthn: Arc<Option<Webauthn>>,
        credentials: Arc<dyn WebauthnCredentialPort>,
        ceremonies: Arc<PasskeyCeremonyStore>,
        users: Arc<dyn UserRepositoryPort>,
        security: Arc<dyn UserSecurityPort>,
        email: Arc<dyn EmailPort>,
    ) -> Self {
        Self { webauthn, credentials, ceremonies, users, security, email }
    }

    /// The `PasskeyAdded` audit entry goes in the same transaction as the credential.
    pub async fn execute(&self, user_id: Uuid, organization_id: Uuid, challenge_id: Uuid, response: &RegisterPublicKeyCredential, name: &str) -> Result<Uuid, ApplicationError> {
        let name = name.trim();
        if name.chars().count() > MAX_PASSKEY_NAME_CHARS {
            return Err(ApplicationError::Domain(DomainError::Validation(format!("a passkey name is at most {MAX_PASSKEY_NAME_CHARS} characters"))));
        }
        ensure_room_for_another_passkey(self.credentials.count_for_user(user_id).await? as usize)?;
        let webauthn = require_webauthn(&self.webauthn)?;
        let entry = self.ceremonies.take(challenge_id, user_id).ok_or(ApplicationError::InvalidMfaCode)?;
        let CeremonyState::Registration(registration) = entry.state else {
            return Err(ApplicationError::InvalidMfaCode);
        };

        let passkey = webauthn.finish_passkey_registration(response, &registration).map_err(|_| ApplicationError::InvalidMfaCode)?;

        let credential = WebauthnCredential { id: Uuid::new_v4(), user_id, name: name.to_string(), passkey_data: serialize_passkey(&passkey), created_at: Utc::now(), last_used_at: None };
        let audit = SecurityAuditRecord { event: SecurityEvent::PasskeyAdded { user_id, organization_id, passkey_id: credential.id }, actor_id: Some(user_id) };
        self.credentials.insert(&credential, Some(&audit)).await?;

        // Best-effort: registration already succeeded, a delivery failure must not undo it.
        if let Ok(Some(user)) = self.users.find_by_id(user_id).await {
            if let Some(email) = crate::use_cases::mfa::verified_address(self.security.as_ref(), &user).await {
                let language = self.email.language_for(user.id).await;
                let content = crate::email_templates::mfa_enrolled(language, user.username.as_str(), crate::email_templates::EnrolledMethod::Passkey);
                if let Err(e) = self.email.send(user.organization_id, &email, &content.subject, &content.text, &content.html).await {
                    tracing::warn!("failed to send passkey-enrollment confirmation email to {email}: {e}");
                }
            }
        }
        Ok(credential.id)
    }
}

pub struct StartPasskeyAuthenticationUseCase {
    webauthn: Arc<Option<Webauthn>>,
    credentials: Arc<dyn WebauthnCredentialPort>,
    ceremonies: Arc<PasskeyCeremonyStore>,
}

impl StartPasskeyAuthenticationUseCase {
    pub fn new(webauthn: Arc<Option<Webauthn>>, credentials: Arc<dyn WebauthnCredentialPort>, ceremonies: Arc<PasskeyCeremonyStore>) -> Self {
        Self { webauthn, credentials, ceremonies }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<(Uuid, RequestChallengeResponse), ApplicationError> {
        let webauthn = require_webauthn(&self.webauthn)?;
        let existing = self.credentials.list_for_user(user_id).await?;
        if existing.is_empty() {
            return Err(ApplicationError::MfaNotEnrolled);
        }
        let passkeys: Vec<Passkey> = existing.iter().map(deserialize_passkey).collect::<Result<_, _>>()?;

        let (rcr, authentication) =
            webauthn.start_passkey_authentication(&passkeys).map_err(|e| ApplicationError::Domain(artiferris_domain::error::DomainError::Infrastructure(e.to_string())))?;

        let challenge_id = self.ceremonies.insert(user_id, CeremonyState::Authentication(authentication));
        Ok((challenge_id, rcr))
    }
}

pub struct FinishPasskeyAuthenticationUseCase {
    webauthn: Arc<Option<Webauthn>>,
    credentials: Arc<dyn WebauthnCredentialPort>,
    ceremonies: Arc<PasskeyCeremonyStore>,
}

impl FinishPasskeyAuthenticationUseCase {
    pub fn new(webauthn: Arc<Option<Webauthn>>, credentials: Arc<dyn WebauthnCredentialPort>, ceremonies: Arc<PasskeyCeremonyStore>) -> Self {
        Self { webauthn, credentials, ceremonies }
    }

    pub async fn execute(&self, user_id: Uuid, challenge_id: Uuid, response: &PublicKeyCredential) -> Result<(), ApplicationError> {
        let webauthn = require_webauthn(&self.webauthn)?;
        let entry = self.ceremonies.take(challenge_id, user_id).ok_or(ApplicationError::InvalidMfaCode)?;
        let CeremonyState::Authentication(authentication) = entry.state else {
            return Err(ApplicationError::InvalidMfaCode);
        };

        let auth_result = webauthn.finish_passkey_authentication(response, &authentication).map_err(|_| ApplicationError::InvalidMfaCode)?;

        for stored in self.credentials.list_for_user(user_id).await? {
            // An unreadable record can't be the one that just verified.
            let Ok(mut passkey) = deserialize_passkey(&stored) else { continue };
            if passkey.cred_id() != auth_result.cred_id() {
                continue;
            }
            if auth_result.needs_update() && passkey.update_credential(&auth_result).unwrap_or(false) {
                self.credentials.update_passkey_data(stored.id, serialize_passkey(&passkey)).await?;
            }
            self.credentials.mark_used(stored.id, Utc::now()).await?;
            break;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeySummary {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub struct ListPasskeysUseCase {
    credentials: Arc<dyn WebauthnCredentialPort>,
}

impl ListPasskeysUseCase {
    pub fn new(credentials: Arc<dyn WebauthnCredentialPort>) -> Self {
        Self { credentials }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<Vec<PasskeySummary>, ApplicationError> {
        let credentials = self.credentials.list_for_user(user_id).await?;
        Ok(credentials.into_iter().map(|c| PasskeySummary { id: c.id, name: c.name, created_at: c.created_at, last_used_at: c.last_used_at }).collect())
    }
}

pub struct DeletePasskeyUseCase {
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    credentials: Arc<dyn WebauthnCredentialPort>,
    totp: Arc<dyn TotpCredentialPort>,
    backup_codes: Arc<dyn BackupCodePort>,
}

impl DeletePasskeyUseCase {
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        hasher: Arc<dyn PasswordHasherPort>,
        credentials: Arc<dyn WebauthnCredentialPort>,
        totp: Arc<dyn TotpCredentialPort>,
        backup_codes: Arc<dyn BackupCodePort>,
    ) -> Self {
        Self { users, hasher, credentials, totp, backup_codes }
    }

    /// `audit` is written in the same transaction as the deletion. The backup codes go with the last factor: first when
    /// nothing else would remain, so a failure halfway never leaves live codes without a factor, and again afterwards
    /// if the other factor was removed at the same time.
    pub async fn execute(&self, user_id: Uuid, credential_id: Uuid, current_password: &str, audit: Option<&SecurityAuditRecord>) -> Result<(), ApplicationError> {
        let user = self.users.find_by_id(user_id).await?.ok_or(ApplicationError::InvalidCredentials)?;
        if !self.hasher.verify(current_password, &user.password_hash).await? {
            return Err(ApplicationError::InvalidCredentials);
        }
        let owned = self.credentials.list_for_user(user_id).await?;
        let other_passkey = owned.iter().any(|c| c.id != credential_id);
        let another_factor_remains = other_passkey || self.totp.get(user_id).await?.is_some_and(|c| c.confirmed);
        if !another_factor_remains && owned.iter().any(|c| c.id == credential_id) {
            self.backup_codes.delete_all(user_id).await?;
        }
        self.credentials.delete(credential_id, user_id, audit).await?;
        if another_factor_remains && !has_any_factor(self.totp.as_ref(), self.credentials.as_ref(), user_id).await? {
            self.backup_codes.delete_all(user_id).await?;
        }
        Ok(())
    }
}

/// Built from `ARTIFERRIS_BASE_DOMAIN`, not `PUBLIC_URL`: `rp_id` needs the shared base domain for
/// `allow_subdomains(true)`. A non-default port is taken from `public_url` rather than defaulting to 80/443.
fn rp_origin_url(artiferris_base_domain: &str, public_url: &str) -> Result<Url, String> {
    let scheme = crate::base_domain::scheme_for_domain(artiferris_base_domain);
    let port_suffix = Url::parse(public_url).ok().and_then(|u| u.port()).map(|p| format!(":{p}")).unwrap_or_default();
    Url::parse(&format!("{scheme}://{artiferris_base_domain}{port_suffix}")).map_err(|e| format!("ARTIFERRIS_BASE_DOMAIN ({artiferris_base_domain}) is not usable as a URL: {e}"))
}

/// Returns `Err` instead of panicking on an unusable `ARTIFERRIS_BASE_DOMAIN`.
pub fn build_webauthn_client(artiferris_base_domain: &str, rp_name: &str, public_url: &str) -> Result<Webauthn, String> {
    let rp_origin = rp_origin_url(artiferris_base_domain, public_url)?;
    WebauthnBuilder::new(artiferris_base_domain, &rp_origin)
        .map_err(|e| format!("ARTIFERRIS_BASE_DOMAIN ({artiferris_base_domain}) is not usable as a WebAuthn relying party: {e}"))?
        .rp_name(rp_name)
        .allow_subdomains(true)
        .build()
        .map_err(|e| format!("failed to build the WebAuthn client: {e}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::user::{User, Username};
    use serde_json::json;

    use super::*;
    use crate::use_cases::verification_test_support::FakeVerification;

    struct FakeCredentials {
        by_user: StdMutex<StdHashMap<Uuid, Vec<WebauthnCredential>>>,
    }

    impl FakeCredentials {
        fn new() -> Self {
            Self { by_user: StdMutex::new(StdHashMap::new()) }
        }
    }

    #[async_trait]
    impl WebauthnCredentialPort for FakeCredentials {
        async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<WebauthnCredential>, DomainError> {
            Ok(self.by_user.lock().unwrap().get(&user_id).cloned().unwrap_or_default())
        }
        async fn insert(&self, credential: &WebauthnCredential, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().entry(credential.user_id).or_default().push(credential.clone());
            Ok(())
        }
        async fn update_passkey_data(&self, id: Uuid, passkey_data: Vec<u8>) -> Result<(), DomainError> {
            for creds in self.by_user.lock().unwrap().values_mut() {
                if let Some(c) = creds.iter_mut().find(|c| c.id == id) {
                    c.passkey_data = passkey_data;
                    return Ok(());
                }
            }
            Ok(())
        }
        async fn mark_used(&self, id: Uuid, at: DateTime<Utc>) -> Result<(), DomainError> {
            for creds in self.by_user.lock().unwrap().values_mut() {
                if let Some(c) = creds.iter_mut().find(|c| c.id == id) {
                    c.last_used_at = Some(at);
                }
            }
            Ok(())
        }
        async fn delete(&self, id: Uuid, user_id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            if let Some(creds) = self.by_user.lock().unwrap().get_mut(&user_id) {
                creds.retain(|c| c.id != id);
            }
            Ok(())
        }
        async fn count_for_user(&self, user_id: Uuid) -> Result<i64, DomainError> {
            Ok(self.by_user.lock().unwrap().get(&user_id).map(|c| c.len()).unwrap_or(0) as i64)
        }
    }

    struct FakeUsers {
        users: StdMutex<StdHashMap<Uuid, User>>,
    }

    impl FakeUsers {
        fn new() -> Self {
            Self { users: StdMutex::new(StdHashMap::new()) }
        }

        fn with_user(user: User) -> Self {
            let mut users = StdHashMap::new();
            users.insert(user.id, user);
            Self { users: StdMutex::new(users) }
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
        async fn update_password(&self, _id: Uuid, _new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> {
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

    struct FakeEmail {
        sent: StdMutex<Vec<(Uuid, String, String, String, String)>>,
    }

    impl FakeEmail {
        fn new() -> Self {
            Self { sent: StdMutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl EmailPort for FakeEmail {
        async fn send(&self, organization_id: Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError> {
            self.sent.lock().unwrap().push((organization_id, to.to_string(), subject.to_string(), text_body.to_string(), html_body.to_string()));
            Ok(())
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

    fn sample_user() -> User {
        User { id: Uuid::new_v4(), username: Username::parse("florian").unwrap(), password_hash: "hashed:s3cret!".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }
    }

    fn test_webauthn() -> Arc<Option<Webauthn>> {
        Arc::new(Some(build_webauthn_client("artiferris.example.com", "ArtiFerris", "https://artiferris.example.com").unwrap()))
    }

    /// Deserializes cleanly but is cryptographically meaningless — good enough for tests that only need `finish_*` to reach (and fail) the crypto check.
    fn fake_register_response() -> RegisterPublicKeyCredential {
        serde_json::from_value(json!({
            "id": "AAAA",
            "rawId": "AAAA",
            "response": { "attestationObject": "", "clientDataJSON": "" },
            "type": "public-key",
        }))
        .unwrap()
    }

    fn fake_auth_response() -> PublicKeyCredential {
        serde_json::from_value(json!({
            "id": "AAAA",
            "rawId": "AAAA",
            "response": { "authenticatorData": "", "clientDataJSON": "", "signature": "" },
            "type": "public-key",
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn starting_registration_returns_a_challenge_id_and_creation_options() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = StartPasskeyRegistrationUseCase::new(test_webauthn(), credentials, ceremonies, users, Arc::new(FakeHasher));

        let (challenge_id, ccr) = use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap();

        assert_ne!(challenge_id, Uuid::nil());
        assert_eq!(ccr.public_key.user.name, "florian");
    }

    /// A hijacked session token alone must not start a passkey registration: the current password is required.
    #[tokio::test]
    async fn starting_registration_requires_the_current_password() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = StartPasskeyRegistrationUseCase::new(test_webauthn(), credentials, ceremonies, users, Arc::new(FakeHasher));

        let err = use_case.execute(user.id, "florian", Some("wrong-password")).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidCredentials));

        use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap();
    }

    /// The first-time setup flow passes `None` and skips the password check.
    #[tokio::test]
    async fn starting_registration_with_no_password_supplied_skips_the_check() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = StartPasskeyRegistrationUseCase::new(test_webauthn(), credentials, ceremonies, users, Arc::new(FakeHasher));

        let (challenge_id, ccr) = use_case.execute(user.id, "florian", None).await.unwrap();

        assert_ne!(challenge_id, Uuid::nil());
        assert_eq!(ccr.public_key.user.name, "florian");
    }

    fn some_registration_state() -> CeremonyState {
        let webauthn = test_webauthn();
        let (_, registration) = require_webauthn(&webauthn).unwrap().start_passkey_registration(Uuid::new_v4(), "florian", "florian", None).unwrap();
        CeremonyState::Registration(registration)
    }

    #[test]
    fn the_oldest_ceremony_of_a_user_is_dropped_past_the_per_user_cap() {
        let store = PasskeyCeremonyStore::with_limits(Duration::minutes(5), 100, 2);
        let user = Uuid::new_v4();
        let first = store.insert(user, some_registration_state());
        let second = store.insert(user, some_registration_state());
        let third = store.insert(user, some_registration_state());

        assert_eq!(store.len(), 2);
        assert!(store.take(first, user).is_none(), "the oldest one made room");
        assert!(store.take(second, user).is_some());
        assert!(store.take(third, user).is_some());
    }

    #[test]
    fn one_users_ceremonies_never_push_out_anothers() {
        let store = PasskeyCeremonyStore::with_limits(Duration::minutes(5), 100, 2);
        let victim = Uuid::new_v4();
        let victims = store.insert(victim, some_registration_state());
        let attacker = Uuid::new_v4();
        for _ in 0..10 {
            store.insert(attacker, some_registration_state());
        }

        assert_eq!(store.len(), 3);
        assert!(store.take(victims, victim).is_some());
    }

    #[test]
    fn the_store_never_grows_past_the_global_cap() {
        let store = PasskeyCeremonyStore::with_limits(Duration::minutes(5), 3, 5);
        let ids: Vec<_> = (0..5)
            .map(|_| {
                let user = Uuid::new_v4();
                (user, store.insert(user, some_registration_state()))
            })
            .collect();

        assert_eq!(store.len(), 3);
        assert!(store.take(ids[0].1, ids[0].0).is_none());
        assert!(store.take(ids[1].1, ids[1].0).is_none());
        assert!(store.take(ids[4].1, ids[4].0).is_some(), "the newest survive");
    }

    #[test]
    fn expired_ceremonies_are_dropped_on_the_next_insert() {
        let store = PasskeyCeremonyStore::with_limits(Duration::milliseconds(-1), 100, 5);
        for _ in 0..4 {
            store.insert(Uuid::new_v4(), some_registration_state());
        }

        assert_eq!(store.len(), 1, "only the entry just inserted is left");
    }

    #[test]
    fn an_expired_ceremony_cannot_be_taken() {
        let store = PasskeyCeremonyStore::with_limits(Duration::milliseconds(-1), 100, 5);
        let user = Uuid::new_v4();
        let id = store.insert(user, some_registration_state());

        assert!(store.take(id, user).is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn taking_a_ceremony_frees_its_slot_in_the_per_user_count() {
        let store = PasskeyCeremonyStore::with_limits(Duration::minutes(5), 100, 2);
        let user = Uuid::new_v4();
        let first = store.insert(user, some_registration_state());
        let second = store.insert(user, some_registration_state());
        store.take(first, user).unwrap();

        let third = store.insert(user, some_registration_state());

        assert!(store.take(second, user).is_some(), "a slot was free, so nothing had to be evicted");
        assert!(store.take(third, user).is_some());
    }

    #[test]
    fn build_webauthn_client_rejects_an_ip_literal_base_domain() {
        let err = build_webauthn_client("0.0.0.0", "ArtiFerris", "http://0.0.0.0:8080").unwrap_err();
        assert!(!err.is_empty());
    }

    /// A bare `scheme://base_domain` assumes the default port and would break passkeys on a non-default one.
    #[test]
    fn rp_origin_includes_a_non_default_port_taken_from_public_url() {
        let origin = rp_origin_url("localhost", "http://localhost:8080").unwrap();
        assert_eq!(origin.as_str(), "http://localhost:8080/");
    }

    #[test]
    fn rp_origin_uses_http_for_a_dotted_localhost_dev_domain() {
        let origin = rp_origin_url("artiferris.localhost", "http://artiferris.localhost:4200").unwrap();
        assert_eq!(origin.as_str(), "http://artiferris.localhost:4200/");
    }

    /// A real PUBLIC_URL is typically just `https://artiferris.example.com`, no port since 443 is the default — the RP origin must match exactly, no spurious `:443`.
    #[test]
    fn rp_origin_omits_the_port_when_public_url_uses_the_schemes_default_port() {
        let origin = rp_origin_url("artiferris.example.com", "https://artiferris.example.com").unwrap();
        assert_eq!(origin.as_str(), "https://artiferris.example.com/");
    }

    /// PUBLIC_URL missing or unparsable must not prevent the server from starting — falls back to no port suffix.
    #[test]
    fn rp_origin_falls_back_to_no_port_when_public_url_is_unusable() {
        let origin = rp_origin_url("localhost", "not a url").unwrap();
        assert_eq!(origin.as_str(), "http://localhost/");
    }

    #[tokio::test]
    async fn passkey_registration_uses_the_shared_base_domain_as_the_relying_party_id() {
        let webauthn = test_webauthn();
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = StartPasskeyRegistrationUseCase::new(webauthn, credentials, ceremonies, users, Arc::new(FakeHasher));

        let (_challenge_id, ccr) = use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap();

        assert_eq!(ccr.public_key.rp.id, "artiferris.example.com");
    }

    #[tokio::test]
    async fn starting_registration_fails_gracefully_when_webauthn_is_unavailable() {
        let webauthn: Arc<Option<Webauthn>> = Arc::new(None);
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let use_case = StartPasskeyRegistrationUseCase::new(webauthn, credentials, ceremonies, users, Arc::new(FakeHasher));

        let err = use_case.execute(user.id, "florian", Some("s3cret!")).await.unwrap_err();
        assert!(matches!(err, ApplicationError::PasskeysUnavailable));
    }

    #[tokio::test]
    async fn finishing_registration_with_an_unknown_challenge_id_fails() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let use_case = FinishPasskeyRegistrationUseCase::new(test_webauthn(), credentials, ceremonies, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()));

        let err = use_case.execute(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), &fake_register_response(), "My key").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn finishing_registration_with_a_challenge_belonging_to_a_different_user_fails() {
        let webauthn = test_webauthn();
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let start = StartPasskeyRegistrationUseCase::new(webauthn.clone(), credentials.clone(), ceremonies.clone(), users, Arc::new(FakeHasher));
        let (challenge_id, _ccr) = start.execute(user.id, "florian", Some("s3cret!")).await.unwrap();

        let finish = FinishPasskeyRegistrationUseCase::new(webauthn, credentials, ceremonies, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()));
        let someone_else = Uuid::new_v4();
        let err = finish.execute(someone_else, Uuid::new_v4(), challenge_id, &fake_register_response(), "My key").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    async fn credential_named(user_id: Uuid, name: &str) -> WebauthnCredential {
        WebauthnCredential { id: Uuid::new_v4(), user_id, name: name.to_string(), passkey_data: b"opaque".to_vec(), created_at: Utc::now(), last_used_at: None }
    }

    #[tokio::test]
    async fn a_passkey_name_past_the_length_limit_is_refused_before_anything_is_checked() {
        let finish = FinishPasskeyRegistrationUseCase::new(
            test_webauthn(),
            Arc::new(FakeCredentials::new()),
            Arc::new(PasskeyCeremonyStore::new()),
            Arc::new(FakeUsers::new()),
            Arc::new(FakeVerification::nobody()),
            Arc::new(FakeEmail::new()),
        );

        let err = finish.execute(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), &fake_register_response(), &"k".repeat(MAX_PASSKEY_NAME_CHARS + 1)).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Domain(DomainError::Validation(_))), "{err:?}");

        let err = finish.execute(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), &fake_register_response(), &format!("  {}  ", "k".repeat(MAX_PASSKEY_NAME_CHARS))).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode), "padding does not count and a name of exactly the limit passes the check: {err:?}");
    }

    #[tokio::test]
    async fn an_account_at_the_passkey_limit_can_neither_start_nor_finish_another_registration() {
        let webauthn = test_webauthn();
        let credentials = Arc::new(FakeCredentials::new());
        let user = sample_user();
        for i in 0..MAX_PASSKEYS_PER_USER {
            credentials.insert(&credential_named(user.id, &format!("key {i}")).await, None).await.unwrap();
        }
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());

        let start = StartPasskeyRegistrationUseCase::new(webauthn.clone(), credentials.clone(), ceremonies.clone(), users, Arc::new(FakeHasher));
        let err = start.execute(user.id, "florian", Some("s3cret!")).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Domain(DomainError::Validation(_))), "{err:?}");

        let finish = FinishPasskeyRegistrationUseCase::new(webauthn, credentials.clone(), ceremonies, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()));
        let err = finish.execute(user.id, Uuid::new_v4(), Uuid::new_v4(), &fake_register_response(), "one more").await.unwrap_err();
        assert!(matches!(err, ApplicationError::Domain(DomainError::Validation(_))), "{err:?}");
        assert_eq!(credentials.count_for_user(user.id).await.unwrap(), MAX_PASSKEYS_PER_USER as i64);
    }

    #[tokio::test]
    async fn a_challenge_id_can_only_be_finished_once() {
        let webauthn = test_webauthn();
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let start = StartPasskeyRegistrationUseCase::new(webauthn.clone(), credentials.clone(), ceremonies.clone(), users, Arc::new(FakeHasher));
        let (challenge_id, _ccr) = start.execute(user_id, "florian", Some("s3cret!")).await.unwrap();

        let finish = FinishPasskeyRegistrationUseCase::new(webauthn, credentials, ceremonies, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()));
        let _ = finish.execute(user_id, Uuid::new_v4(), challenge_id, &fake_register_response(), "My key").await;
        let err = finish.execute(user_id, Uuid::new_v4(), challenge_id, &fake_register_response(), "My key").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn finishing_registration_with_a_cryptographically_invalid_response_fails() {
        let webauthn = test_webauthn();
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let user_id = user.id;
        let start = StartPasskeyRegistrationUseCase::new(webauthn.clone(), credentials.clone(), ceremonies.clone(), users, Arc::new(FakeHasher));
        let (challenge_id, _ccr) = start.execute(user_id, "florian", Some("s3cret!")).await.unwrap();

        let finish = FinishPasskeyRegistrationUseCase::new(webauthn, credentials.clone(), ceremonies, Arc::new(FakeUsers::new()), Arc::new(FakeVerification::nobody()), Arc::new(FakeEmail::new()));
        let err = finish.execute(user_id, Uuid::new_v4(), challenge_id, &fake_register_response(), "My key").await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidMfaCode));
        assert_eq!(credentials.count_for_user(user_id).await.unwrap(), 0, "a failed registration must not persist a credential");
    }

    #[tokio::test]
    async fn starting_authentication_fails_when_no_passkeys_are_registered() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let use_case = StartPasskeyAuthenticationUseCase::new(test_webauthn(), credentials, ceremonies);

        let err = use_case.execute(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::MfaNotEnrolled));
    }

    #[tokio::test]
    async fn finishing_authentication_with_an_unknown_challenge_id_fails() {
        let credentials = Arc::new(FakeCredentials::new());
        let ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let use_case = FinishPasskeyAuthenticationUseCase::new(test_webauthn(), credentials, ceremonies);

        let err = use_case.execute(Uuid::new_v4(), Uuid::new_v4(), &fake_auth_response()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn list_passkeys_returns_the_users_registered_credentials() {
        let credentials = Arc::new(FakeCredentials::new());
        let user_id = Uuid::new_v4();
        credentials.insert(&WebauthnCredential { id: Uuid::new_v4(), user_id, name: "MacBook".to_string(), passkey_data: b"opaque".to_vec(), created_at: Utc::now(), last_used_at: None }, None).await.unwrap();

        let summaries = ListPasskeysUseCase::new(credentials).execute(user_id).await.unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "MacBook");
    }

    #[tokio::test]
    async fn deleting_a_passkey_requires_the_current_password() {
        let credentials = Arc::new(FakeCredentials::new());
        let user = sample_user();
        let users = Arc::new(FakeUsers::with_user(user.clone()));
        let credential_id = Uuid::new_v4();
        credentials.insert(&WebauthnCredential { id: credential_id, user_id: user.id, name: "MacBook".to_string(), passkey_data: b"opaque".to_vec(), created_at: Utc::now(), last_used_at: None }, None).await.unwrap();

        let use_case = DeletePasskeyUseCase::new(users, Arc::new(FakeHasher), credentials.clone(), Arc::new(FakeTotp(false)), Arc::new(FakeBackupCodes::default()));
        let err = use_case.execute(user.id, credential_id, "wrong-password", None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidCredentials));
        assert_eq!(credentials.count_for_user(user.id).await.unwrap(), 1);

        use_case.execute(user.id, credential_id, "s3cret!", None).await.unwrap();
        assert_eq!(credentials.count_for_user(user.id).await.unwrap(), 0);
    }

    /// Whether the account has a confirmed authenticator app; nothing else is asked of it here.
    struct FakeTotp(bool);

    #[async_trait]
    impl TotpCredentialPort for FakeTotp {
        async fn get(&self, user_id: Uuid) -> Result<Option<artiferris_domain::mfa::TotpCredential>, DomainError> {
            Ok(self.0.then(|| artiferris_domain::mfa::TotpCredential { user_id, secret: "S".to_string(), confirmed: true, last_used_step: None, created_at: Utc::now() }))
        }
        async fn begin_enrollment(&self, _user_id: Uuid, _secret: &str, _created_at: DateTime<Utc>) -> Result<bool, DomainError> {
            Ok(false)
        }
        async fn confirm(&self, _user_id: Uuid, _enrollment_created_at: DateTime<Utc>, _step: i64) -> Result<bool, DomainError> {
            Ok(false)
        }
        async fn set_last_used_step(&self, _user_id: Uuid, _step: i64) -> Result<bool, DomainError> {
            Ok(false)
        }
        async fn delete(&self, _user_id: Uuid) -> Result<(), DomainError> {
            Ok(())
        }
    }

    /// Only how many codes are left.
    #[derive(Default)]
    struct FakeBackupCodes(StdMutex<i64>);

    #[async_trait]
    impl BackupCodePort for FakeBackupCodes {
        async fn replace_all(&self, _user_id: Uuid, code_hashes: &[String], _audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
            *self.0.lock().unwrap() = code_hashes.len() as i64;
            Ok(())
        }
        async fn try_consume(&self, _user_id: Uuid, _plaintext_code: &str) -> Result<bool, DomainError> {
            Ok(false)
        }
        async fn count_unused(&self, _user_id: Uuid) -> Result<i64, DomainError> {
            Ok(*self.0.lock().unwrap())
        }
        async fn delete_all(&self, _user_id: Uuid) -> Result<(), DomainError> {
            *self.0.lock().unwrap() = 0;
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_backup_codes_go_with_the_last_factor_only() {
        for (other_passkey, app, codes_left) in [(false, false, 0), (true, false, 10), (false, true, 10)] {
            let credentials = Arc::new(FakeCredentials::new());
            let user = sample_user();
            let removed = credential_named(user.id, "MacBook").await;
            credentials.insert(&removed, None).await.unwrap();
            if other_passkey {
                credentials.insert(&credential_named(user.id, "YubiKey").await, None).await.unwrap();
            }
            let backup_codes = Arc::new(FakeBackupCodes::default());
            backup_codes.replace_all(user.id, &vec!["h".to_string(); 10], None).await.unwrap();

            DeletePasskeyUseCase::new(Arc::new(FakeUsers::with_user(user.clone())), Arc::new(FakeHasher), credentials, Arc::new(FakeTotp(app)), backup_codes.clone())
                .execute(user.id, removed.id, "s3cret!", None)
                .await
                .unwrap();

            assert_eq!(backup_codes.count_unused(user.id).await.unwrap(), codes_left, "other passkey {other_passkey}, app {app}");
        }
    }
}
