use std::sync::Arc;

use chrono::{Duration, Utc};
use artiferris_domain::audit::{AdminAuditEvent, AdminAuditRecord, AuditRecord};
use artiferris_domain::email::EmailPort;
use artiferris_domain::error::DomainError;
use artiferris_domain::invitation::{UserInvitation, UserInvitationPort};
use artiferris_domain::organization::{Organization, OrganizationRepositoryPort};
use artiferris_domain::user::{Password, PasswordHasherPort, User, UserRepositoryPort, UserSecurityPort, Username};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApplicationError;

/// `https://<slug>.<base_domain>`, or `https://app.<base_domain>` for the public organization: where an invited user's
/// organization is served.
pub fn organization_origin(artiferris_base_domain: &str, organization: &Organization) -> String {
    let host = if organization.is_public { format!("app.{}", artiferris_base_domain) } else { format!("{}.{}", organization.slug.as_str(), artiferris_base_domain) };
    format!("{}://{}", crate::base_domain::scheme_for_domain(artiferris_base_domain), host)
}

pub(crate) async fn require_organization(organizations: &dyn OrganizationRepositoryPort, organization_id: Uuid) -> Result<Organization, ApplicationError> {
    organizations
        .find_by_id(organization_id)
        .await?
        .ok_or_else(|| ApplicationError::Domain(DomainError::Infrastructure(format!("organization {organization_id} not found"))))
}

pub(crate) const INVITATION_TTL_HOURS: i64 = 24;

pub(crate) fn generate_invitation_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// `pub` so tests elsewhere can seed a `UserInvitation` with a known token hash.
pub fn hash_invitation_token(plaintext: &str) -> String {
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

/// Hashes fresh random bytes so `AuthenticateUserUseCase` needs no special case for a pending account — a normal failed `verify` already rejects it.
pub(crate) async fn unusable_password_hash(hasher: &dyn PasswordHasherPort) -> Result<String, ApplicationError> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    Ok(hasher.hash(&hex::encode(bytes)).await?)
}

const PLACEHOLDER_USERNAME_PREFIX: &str = "invite-";

/// RFC 5321's limit on a whole address.
const MAX_EMAIL_LEN: usize = 254;

pub(crate) fn validate_email(email: &str) -> Result<(), ApplicationError> {
    let trimmed = email.trim();
    if trimmed.is_empty() || trimmed != email || email.len() > MAX_EMAIL_LEN || email.chars().any(|c| c.is_control()) {
        return Err(ApplicationError::InvalidEmail(email.to_string()));
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(ApplicationError::InvalidEmail(email.to_string()));
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') || email.chars().any(char::is_whitespace) {
        return Err(ApplicationError::InvalidEmail(email.to_string()));
    }
    Ok(())
}

pub struct InviteUserUseCase {
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    invitations: Arc<dyn UserInvitationPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    email: Arc<dyn EmailPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    /// No trailing slash. The base domain organization subdomains are resolved against — see `organization_origin` above.
    artiferris_base_domain: String,
}

impl InviteUserUseCase {
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        security: Arc<dyn UserSecurityPort>,
        invitations: Arc<dyn UserInvitationPort>,
        hasher: Arc<dyn PasswordHasherPort>,
        email: Arc<dyn EmailPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
        artiferris_base_domain: String,
    ) -> Self {
        Self { users, security, invitations, hasher, email, organizations, artiferris_base_domain }
    }

    /// `organization_id` is normally the acting admin's own org, but a super-admin inviting an org's first local admin can pass any organization.
    /// The invitee chooses their username when activating, so the pending account holds a placeholder until then.
    /// The `UserInvited` audit entry is written with the invitation; if that fails, the account is removed again.
    pub async fn execute(&self, organization_id: Uuid, is_organization_admin: bool, email: &str, is_super_admin: bool, actor_id: Uuid) -> Result<Uuid, ApplicationError> {
        validate_email(email)?;
        if self.security.find_by_verified_email(organization_id, email).await?.is_some() {
            return Err(DomainError::EmailTaken.into());
        }

        let user = User {
            id: Uuid::new_v4(),
            username: self.placeholder_username().await?,
            password_hash: unusable_password_hash(self.hasher.as_ref()).await?,
            is_super_admin,
            is_organization_admin,
            organization_id,
            created_at: Utc::now(),
            tokens_valid_after: Utc::now(),
            email: Some(email.to_string()),
        };
        // Not verified yet: nobody has proven the address until the invitation is redeemed.
        self.users.insert(&user).await?;

        let token = generate_invitation_token();
        let audit = AdminAuditRecord {
            event: AdminAuditEvent::UserInvited { user_id: user.id, organization_id, email: email.to_string(), is_organization_admin, is_super_admin },
            actor_id: Some(actor_id),
        };
        let invitation = UserInvitation { user_id: user.id, token_hash: hash_invitation_token(&token), expires_at: Utc::now() + Duration::hours(INVITATION_TTL_HOURS) };
        if let Err(e) = self.invitations.upsert(&invitation, Some(&audit)).await {
            if let Err(cleanup) = self.users.delete(user.id).await {
                tracing::error!("failed to remove user {} after its invitation could not be stored: {cleanup}", user.id);
            }
            return Err(e.into());
        }

        // After persisting the user: if the org lookup fails, the account exists and `ResendInvitationUseCase` can
        // reach it.
        let organization = require_organization(self.organizations.as_ref(), organization_id).await?;
        let origin = organization_origin(&self.artiferris_base_domain, &organization);

        // A delivery failure does not fail account creation.
        let activation_url = format!("{origin}/activate?token={token}");
        // The invitee cannot have chosen a language yet: write in the one of the admin who invites them.
        let language = self.email.language_for(actor_id).await;
        let content = crate::email_templates::account_created(language, &activation_url);
        if let Err(e) = self.email.send(organization_id, email, &content.subject, &content.text, &content.html).await {
            tracing::warn!("failed to send account-activation email to {email}: {e}");
        }

        Ok(user.id)
    }

    /// A free `invite-<12 hex>` name, valid as a username and never shown once the invitee has chosen their own.
    async fn placeholder_username(&self) -> Result<Username, ApplicationError> {
        for _ in 0..5 {
            let mut bytes = [0u8; 6];
            rand::rng().fill_bytes(&mut bytes);
            let candidate = Username::parse(&format!("{PLACEHOLDER_USERNAME_PREFIX}{}", hex::encode(bytes)))?;
            if self.users.find_by_username(&candidate).await?.is_none() {
                return Ok(candidate);
            }
        }
        Err(ApplicationError::Domain(DomainError::Infrastructure("no free placeholder username".to_string())))
    }
}

pub struct ResendInvitationUseCase {
    users: Arc<dyn UserRepositoryPort>,
    invitations: Arc<dyn UserInvitationPort>,
    email: Arc<dyn EmailPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    artiferris_base_domain: String,
}

impl ResendInvitationUseCase {
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        invitations: Arc<dyn UserInvitationPort>,
        email: Arc<dyn EmailPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
        artiferris_base_domain: String,
    ) -> Self {
        Self { users, invitations, email, organizations, artiferris_base_domain }
    }

    /// An already-activated account has no invitation row left, so this also returns `InvitationNotFound` for it, same as for an unknown user id.
    /// The `InvitationResent` audit entry is written with the new invitation.
    pub async fn execute(&self, user_id: Uuid, actor_id: Uuid) -> Result<(), ApplicationError> {
        let user = self.users.find_by_id(user_id).await?.ok_or(ApplicationError::InvitationNotFound)?;
        if self.invitations.find_by_user_id(user_id).await?.is_none() {
            return Err(ApplicationError::InvitationNotFound);
        }
        let Some(email) = user.email.as_deref() else {
            return Err(ApplicationError::InvitationNotFound);
        };

        let token = generate_invitation_token();
        let audit = AdminAuditRecord { event: AdminAuditEvent::InvitationResent { user_id, organization_id: user.organization_id }, actor_id: Some(actor_id) };
        self.invitations.upsert(&UserInvitation { user_id, token_hash: hash_invitation_token(&token), expires_at: Utc::now() + Duration::hours(INVITATION_TTL_HOURS) }, Some(&audit)).await?;

        // The user's own organization, not whatever the acting admin resolved against.
        let organization = require_organization(self.organizations.as_ref(), user.organization_id).await?;
        let origin = organization_origin(&self.artiferris_base_domain, &organization);
        let activation_url = format!("{origin}/activate?token={token}");
        let language = self.email.language_for(actor_id).await;
        let content = crate::email_templates::account_created(language, &activation_url);
        self.email.send(user.organization_id, email, &content.subject, &content.text, &content.html).await?;
        Ok(())
    }
}

pub struct ActivateAccountUseCase {
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    invitations: Arc<dyn UserInvitationPort>,
    hasher: Arc<dyn PasswordHasherPort>,
}

impl ActivateAccountUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, security: Arc<dyn UserSecurityPort>, invitations: Arc<dyn UserInvitationPort>, hasher: Arc<dyn PasswordHasherPort>) -> Self {
        Self { users, security, invitations, hasher }
    }

    /// `username` is the name the invitee chose. It is checked before the invitation is redeemed, and again by the
    /// database at write time, so a name taken in between still ends in `UsernameTaken`. Returns the id of the account
    /// that was activated.
    pub async fn execute(&self, token: &str, username: &str, new_password: &str) -> Result<Uuid, ApplicationError> {
        let invitation = self.invitations.find_by_token_hash(&hash_invitation_token(token)).await?.ok_or(ApplicationError::InvitationNotFound)?;
        if invitation.expires_at < Utc::now() {
            return Err(ApplicationError::InvitationExpired);
        }
        let username = Username::parse_new(username)?;
        let password = Password::parse(new_password)?;
        let user = self.users.find_by_id(invitation.user_id).await?.ok_or(ApplicationError::InvitationNotFound)?;
        if user.username != username && self.users.find_by_username(&username).await?.is_some() {
            return Err(ApplicationError::UsernameTaken);
        }
        let hash = self.hasher.hash(password.as_str()).await?;

        // Of several parallel activations only one gets the invitation.
        let redeemed = self.invitations.redeem(&invitation.token_hash).await?.ok_or(ApplicationError::InvitationNotFound)?;
        // Put the invitation back on failure so the mailed link still works.
        if let Err(e) = self.set_username_password_and_verify_email(redeemed.user_id, &username, hash).await {
            if let Err(restore_error) = self.invitations.upsert(&redeemed, None).await {
                tracing::error!("failed to restore invitation of user {} after a failed activation: {restore_error}", redeemed.user_id);
            }
            return Err(e);
        }
        Ok(redeemed.user_id)
    }

    /// Redeeming the invitation is what proves the address, so it's verified here and not at invite time.
    /// The `UserActivated` audit entry is written with the username and the password.
    async fn set_username_password_and_verify_email(&self, user_id: Uuid, username: &Username, password_hash: String) -> Result<(), ApplicationError> {
        let user = self.users.find_by_id(user_id).await?.ok_or(ApplicationError::InvitationNotFound)?;
        let audit = AuditRecord::Admin(AdminAuditRecord { event: AdminAuditEvent::UserActivated { user_id, organization_id: user.organization_id }, actor_id: Some(user_id) });
        self.security.activate_invited(user_id, username, password_hash, Some(&audit)).await?;
        self.security.mark_email_verified(user_id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::organization::{OrganizationRepositoryPort, OrganizationSlug};

    use super::*;

    const TEST_BASE_DOMAIN: &str = "artiferris.example.com";

    struct FakeUsers {
        users: Mutex<HashMap<Uuid, User>>,
        verified: Mutex<HashSet<Uuid>>,
        fail_update_password: std::sync::atomic::AtomicBool,
    }

    impl FakeUsers {
        fn new() -> Self {
            Self { users: Mutex::new(HashMap::new()), verified: Mutex::new(HashSet::new()), fail_update_password: std::sync::atomic::AtomicBool::new(false) }
        }
    }

    #[async_trait]
    impl UserSecurityPort for FakeUsers {
        async fn revoke_sessions(&self, _id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            Ok(())
        }
        async fn find_by_verified_email(&self, organization_id: Uuid, email: &str) -> Result<Option<User>, DomainError> {
            let verified = self.verified.lock().unwrap();
            Ok(self.users.lock().unwrap().values().find(|u| u.organization_id == organization_id && verified.contains(&u.id) && u.email.as_deref() == Some(email)).cloned())
        }
        async fn insert_with_verified_email(&self, user: &User) -> Result<(), DomainError> {
            self.users.lock().unwrap().insert(user.id, user.clone());
            self.verified.lock().unwrap().insert(user.id);
            Ok(())
        }
        async fn activate_invited(&self, id: Uuid, username: &Username, new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> {
            if self.fail_update_password.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("update failed".to_string()));
            }
            let mut users = self.users.lock().unwrap();
            if users.values().any(|u| u.id != id && &u.username == username) {
                return Err(DomainError::UsernameTaken);
            }
            if let Some(user) = users.get_mut(&id) {
                user.username = username.clone();
                user.password_hash = new_password_hash;
            }
            Ok(())
        }
        async fn mark_email_verified(&self, id: Uuid) -> Result<bool, DomainError> {
            Ok(self.verified.lock().unwrap().insert(id))
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
            if self.fail_update_password.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("update failed".to_string()));
            }
            if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
                user.password_hash = new_password_hash;
            }
            Ok(())
        }
        async fn set_super_admin(&self, id: Uuid, is_super_admin: bool) -> Result<(), DomainError> {
            if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
                user.is_super_admin = is_super_admin;
            }
            Ok(())
        }
        async fn set_organization_admin(&self, id: Uuid, is_organization_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
                user.is_organization_admin = is_organization_admin;
            }
            Ok(())
        }
        async fn delete_unless_last_super_admin(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            self.users.lock().unwrap().remove(&id);
            Ok(true)
        }
        async fn set_super_admin_unless_last(&self, id: Uuid, is_super_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
                user.is_super_admin = is_super_admin;
            }
            Ok(true)
        }
    }

    struct FakeInvitations {
        by_user: Mutex<HashMap<Uuid, UserInvitation>>,
    }

    impl FakeInvitations {
        fn new() -> Self {
            Self { by_user: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl UserInvitationPort for FakeInvitations {
        async fn upsert(&self, invitation: &UserInvitation, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().insert(invitation.user_id, invitation.clone());
            Ok(())
        }
        async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError> {
            Ok(self.by_user.lock().unwrap().values().find(|i| i.token_hash == token_hash).cloned())
        }
        async fn find_by_user_id(&self, user_id: Uuid) -> Result<Option<UserInvitation>, DomainError> {
            Ok(self.by_user.lock().unwrap().get(&user_id).cloned())
        }
        async fn list_pending_user_ids(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
            let by_user = self.by_user.lock().unwrap();
            Ok(user_ids.iter().filter(|id| by_user.contains_key(id)).copied().collect())
        }
        async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().remove(&user_id);
            Ok(())
        }
        async fn redeem(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError> {
            let mut by_user = self.by_user.lock().unwrap();
            let Some(user_id) = by_user.values().find(|i| i.token_hash == token_hash && i.expires_at > Utc::now()).map(|i| i.user_id) else {
                return Ok(None);
            };
            Ok(by_user.remove(&user_id))
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

    struct FakeOrganizations {
        by_id: Mutex<HashMap<Uuid, Organization>>,
    }

    impl FakeOrganizations {
        fn new() -> Self {
            Self { by_id: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl OrganizationRepositoryPort for FakeOrganizations {
        async fn create(&self, org: &Organization) -> Result<(), DomainError> {
            self.by_id.lock().unwrap().insert(org.id, org.clone());
            Ok(())
        }
        async fn find_by_id(&self, id: Uuid) -> Result<Option<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().get(&id).cloned())
        }
        async fn find_by_slug(&self, slug: &OrganizationSlug) -> Result<Option<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().values().find(|o| &o.slug == slug).cloned())
        }
        async fn find_public(&self) -> Result<Organization, DomainError> {
            self.by_id.lock().unwrap().values().find(|o| o.is_public).cloned().ok_or_else(|| DomainError::Infrastructure("no public organization seeded".to_string()))
        }
        async fn list_all(&self) -> Result<Vec<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().values().cloned().collect())
        }
    }

    /// Seeds one public organization on `TEST_BASE_DOMAIN`, for tests that don't care about multi-tenant routing specifically.
    async fn setup() -> (Arc<FakeUsers>, Arc<FakeInvitations>, Arc<FakeHasher>, Arc<FakeEmail>, Arc<FakeOrganizations>, Uuid) {
        let organizations = Arc::new(FakeOrganizations::new());
        let organization_id = Uuid::new_v4();
        organizations
            .create(&Organization { id: organization_id, slug: OrganizationSlug::parse("public").unwrap(), display_name: "Public".to_string(), is_public: true, is_personal: false, created_at: Utc::now() })
            .await
            .unwrap();
        (Arc::new(FakeUsers::new()), Arc::new(FakeInvitations::new()), Arc::new(FakeHasher), Arc::new(FakeEmail::new()), organizations, organization_id)
    }

    #[tokio::test]
    async fn invites_a_user_and_sends_an_activation_email() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher, email.clone(), organizations, TEST_BASE_DOMAIN.to_string());

        let id = use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        let user = users.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(user.email.as_deref(), Some("florian@example.com"));
        assert!(invitations.find_by_user_id(id).await.unwrap().is_some());

        let sent = email.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, organization_id);
        assert_eq!(sent[0].1, "florian@example.com");
        assert!(sent[0].3.contains("https://app.artiferris.example.com/activate?token="));
    }

    #[tokio::test]
    async fn an_invited_users_email_is_not_verified_before_activation() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations, TEST_BASE_DOMAIN.to_string());

        use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        assert!(users.find_by_verified_email(organization_id, "florian@example.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_email_verified_in_the_organization_cannot_be_invited_again() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations, TEST_BASE_DOMAIN.to_string());
        let first = use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        users.mark_email_verified(first).await.unwrap();

        let err = use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap_err();

        assert!(matches!(err, ApplicationError::Domain(DomainError::EmailTaken)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_email_verified_in_another_organization_can_still_be_invited() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations.clone(), TEST_BASE_DOMAIN.to_string());
        let other_organization_id = Uuid::new_v4();
        organizations.create(&Organization { id: other_organization_id, slug: OrganizationSlug::parse("acme").unwrap(), display_name: "Acme".to_string(), is_public: false, is_personal: false, created_at: Utc::now() }).await.unwrap();
        let first = use_case.execute(other_organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        users.mark_email_verified(first).await.unwrap();

        use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn invites_a_user_into_the_given_organization() {
        let (users, invitations, hasher, email, organizations, _default_organization_id) = setup().await;
        let organization_id = Uuid::new_v4();
        organizations
            .create(&Organization { id: organization_id, slug: OrganizationSlug::parse("acme").unwrap(), display_name: "Acme".to_string(), is_public: false, is_personal: false, created_at: Utc::now() })
            .await
            .unwrap();
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations, TEST_BASE_DOMAIN.to_string());

        let id = use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        let user = users.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(user.organization_id, organization_id);
    }

    #[tokio::test]
    async fn invites_a_user_into_a_non_public_organization_links_to_that_organizations_own_subdomain() {
        let (users, invitations, hasher, email, organizations, _default_organization_id) = setup().await;
        let organization_id = Uuid::new_v4();
        organizations
            .create(&Organization { id: organization_id, slug: OrganizationSlug::parse("acme").unwrap(), display_name: "Acme".to_string(), is_public: false, is_personal: false, created_at: Utc::now() })
            .await
            .unwrap();
        let use_case = InviteUserUseCase::new(users.clone(), users, invitations, hasher, email.clone(), organizations, TEST_BASE_DOMAIN.to_string());

        use_case.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        let sent = email.sent.lock().unwrap();
        assert!(sent[0].3.contains("https://acme.artiferris.example.com/activate?token="), "expected the acme subdomain, got: {}", sent[0].3);
    }

    #[tokio::test]
    async fn an_invited_account_holds_a_placeholder_username_until_activation() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations, TEST_BASE_DOMAIN.to_string());

        let first = use_case.execute(organization_id, false, "a@example.com", false, Uuid::new_v4()).await.unwrap();
        let second = use_case.execute(organization_id, false, "b@example.com", false, Uuid::new_v4()).await.unwrap();

        let first = users.find_by_id(first).await.unwrap().unwrap();
        let second = users.find_by_id(second).await.unwrap().unwrap();
        assert!(first.username.as_str().starts_with(PLACEHOLDER_USERNAME_PREFIX));
        assert_ne!(first.username, second.username);
    }

    #[tokio::test]
    async fn rejects_an_invalid_email() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users, invitations, hasher, email, organizations, TEST_BASE_DOMAIN.to_string());

        let err = use_case.execute(organization_id, false, "not-an-email", false, Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidEmail(_)));
    }

    #[tokio::test]
    async fn an_invited_user_cannot_log_in_before_activating() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher.clone(), email, organizations, TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        let user = users.find_by_id(id).await.unwrap().unwrap();
        assert!(!hasher.verify("anything", &user.password_hash).await.unwrap(), "no plaintext should verify against the placeholder hash");
    }

    /// Recovers the token the same way a real invitee would: from the sent email's link.
    fn extract_token_from_last_email(email: &FakeEmail) -> String {
        let body = email.sent.lock().unwrap().last().unwrap().3.clone();
        body.split("token=").nth(1).unwrap().split_whitespace().next().unwrap().to_string()
    }

    #[tokio::test]
    async fn activates_an_account_with_a_valid_token() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        let token = extract_token_from_last_email(&email);

        let activate = ActivateAccountUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone());
        let activated = activate.execute(&token, "florian", "new-s3cret!").await.unwrap();

        assert_eq!(activated, id);
        let user = users.find_by_id(id).await.unwrap().unwrap();
        assert!(hasher.verify("new-s3cret!", &user.password_hash).await.unwrap());
        assert!(invitations.find_by_user_id(id).await.unwrap().is_none(), "the invitation must be consumed after activation");
    }

    #[tokio::test]
    async fn activation_verifies_the_invited_email() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        let token = extract_token_from_last_email(&email);

        ActivateAccountUseCase::new(users.clone(), users.clone(), invitations, hasher).execute(&token, "florian", "new-s3cret!").await.unwrap();

        assert_eq!(users.find_by_verified_email(organization_id, "florian@example.com").await.unwrap().map(|u| u.id), Some(id));
    }

    #[tokio::test]
    async fn two_parallel_activations_with_one_token_let_exactly_one_through() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        let token = extract_token_from_last_email(&email);
        let activate = Arc::new(ActivateAccountUseCase::new(users.clone(), users, invitations, hasher));

        let (first, second) = tokio::join!(
            { let activate = activate.clone(); let token = token.clone(); tokio::spawn(async move { activate.execute(&token, "florian", "first-s3cret!").await }) },
            { let activate = activate.clone(); let token = token.clone(); tokio::spawn(async move { activate.execute(&token, "florian", "second-s3cret!").await }) },
        );
        let results = [first.unwrap(), second.unwrap()];

        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
        assert!(results.iter().any(|r| matches!(r, Err(ApplicationError::InvitationNotFound))));
    }

    #[tokio::test]
    async fn a_failed_activation_leaves_the_link_usable() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        let token = extract_token_from_last_email(&email);
        let activate = ActivateAccountUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher);

        users.fail_update_password.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(activate.execute(&token, "florian", "new-s3cret!").await.is_err());
        assert!(invitations.find_by_user_id(id).await.unwrap().is_some(), "the invitation must survive a failure between redeeming and setting the password");

        users.fail_update_password.store(false, std::sync::atomic::Ordering::SeqCst);
        activate.execute(&token, "florian", "new-s3cret!").await.unwrap();
    }

    async fn invite_and_token(users: &Arc<FakeUsers>, invitations: &Arc<FakeInvitations>, hasher: &Arc<FakeHasher>, email: &Arc<FakeEmail>, organizations: Arc<FakeOrganizations>, organization_id: Uuid, address: &str) -> (Uuid, String) {
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, address, false, Uuid::new_v4()).await.unwrap();
        (id, extract_token_from_last_email(email))
    }

    #[tokio::test]
    async fn activation_sets_the_username_the_invitee_chose() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let (id, token) = invite_and_token(&users, &invitations, &hasher, &email, organizations, organization_id, "florian@example.com").await;

        ActivateAccountUseCase::new(users.clone(), users.clone(), invitations, hasher).execute(&token, "Florian", "new-s3cret!").await.unwrap();

        assert_eq!(users.find_by_id(id).await.unwrap().unwrap().username.as_str(), "florian");
    }

    #[tokio::test]
    async fn activation_refuses_a_username_another_account_holds_and_keeps_the_link_usable() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let (_, first_token) = invite_and_token(&users, &invitations, &hasher, &email, organizations.clone(), organization_id, "a@example.com").await;
        let activate = ActivateAccountUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone());
        activate.execute(&first_token, "florian", "new-s3cret!").await.unwrap();
        let (second_id, second_token) = invite_and_token(&users, &invitations, &hasher, &email, organizations, organization_id, "b@example.com").await;

        let err = activate.execute(&second_token, "FLORIAN", "new-s3cret!").await.unwrap_err();

        assert!(matches!(err, ApplicationError::UsernameTaken), "got {err:?}");
        assert!(invitations.find_by_user_id(second_id).await.unwrap().is_some(), "the invitation must survive a refused username");
        activate.execute(&second_token, "florian2", "new-s3cret!").await.unwrap();
    }

    #[tokio::test]
    async fn activation_rejects_an_invalid_or_reserved_username() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let (_, token) = invite_and_token(&users, &invitations, &hasher, &email, organizations, organization_id, "a@example.com").await;
        let activate = ActivateAccountUseCase::new(users.clone(), users, invitations, hasher);

        let reserved = activate.execute(&token, "ArtiFerris-docker", "new-s3cret!").await.unwrap_err();
        let short = activate.execute(&token, "ab", "new-s3cret!").await.unwrap_err();

        assert!(matches!(reserved, ApplicationError::Domain(DomainError::ReservedName(_))), "got {reserved:?}");
        assert!(matches!(short, ApplicationError::Domain(DomainError::InvalidUsername(_))), "got {short:?}");
    }

    #[tokio::test]
    async fn rejects_an_unknown_token() {
        let (users, invitations, hasher, _email, _organizations, _organization_id) = setup().await;
        let activate = ActivateAccountUseCase::new(users.clone(), users, invitations, hasher);

        let err = activate.execute("not-a-real-token", "florian", "new-s3cret!").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvitationNotFound));
    }

    #[tokio::test]
    async fn rejects_an_expired_token() {
        let (users, invitations, hasher, _email, _organizations, organization_id) = setup().await;
        let user_id = Uuid::new_v4();
        users
            .insert(&User {
                id: user_id,
                username: Username::parse("florian").unwrap(),
                password_hash: "placeholder".to_string(),
                is_super_admin: false,
                is_organization_admin: false,
                organization_id,
                created_at: Utc::now(),
                tokens_valid_after: Utc::now(),
                email: Some("florian@example.com".to_string()),
            })
            .await
            .unwrap();
        invitations.upsert(&UserInvitation { user_id, token_hash: hash_invitation_token("raw-token"), expires_at: Utc::now() - Duration::hours(1) }, None).await.unwrap();

        let activate = ActivateAccountUseCase::new(users.clone(), users, invitations, hasher);
        let err = activate.execute("raw-token", "florian", "new-s3cret!").await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvitationExpired));
    }

    #[tokio::test]
    async fn resends_an_invitation_with_a_fresh_token() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations.clone(), TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        let first_token_hash = invitations.find_by_user_id(id).await.unwrap().unwrap().token_hash;

        let resend = ResendInvitationUseCase::new(users, invitations.clone(), email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        resend.execute(id, Uuid::new_v4()).await.unwrap();

        let second_token_hash = invitations.find_by_user_id(id).await.unwrap().unwrap().token_hash;
        assert_ne!(first_token_hash, second_token_hash);
        assert_eq!(email.sent.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn resending_for_an_unknown_user_fails() {
        let (users, invitations, _hasher, email, organizations, _organization_id) = setup().await;
        let resend = ResendInvitationUseCase::new(users, invitations, email, organizations, TEST_BASE_DOMAIN.to_string());

        let err = resend.execute(Uuid::new_v4(), Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvitationNotFound));
    }

    #[tokio::test]
    async fn resending_for_an_already_activated_user_fails() {
        let (users, invitations, hasher, email, organizations, organization_id) = setup().await;
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher.clone(), email.clone(), organizations.clone(), TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();
        invitations.delete(id).await.unwrap(); // simulates a completed activation

        let resend = ResendInvitationUseCase::new(users, invitations, email, organizations, TEST_BASE_DOMAIN.to_string());
        let err = resend.execute(id, Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvitationNotFound));
    }

    #[tokio::test]
    async fn resending_links_to_the_invitees_own_organization_not_a_default_one() {
        let (users, invitations, hasher, email, organizations, _default_organization_id) = setup().await;
        let organization_id = Uuid::new_v4();
        organizations
            .create(&Organization { id: organization_id, slug: OrganizationSlug::parse("acme").unwrap(), display_name: "Acme".to_string(), is_public: false, is_personal: false, created_at: Utc::now() })
            .await
            .unwrap();
        let invite = InviteUserUseCase::new(users.clone(), users.clone(), invitations.clone(), hasher, email.clone(), organizations.clone(), TEST_BASE_DOMAIN.to_string());
        let id = invite.execute(organization_id, false, "florian@example.com", false, Uuid::new_v4()).await.unwrap();

        let resend = ResendInvitationUseCase::new(users, invitations, email.clone(), organizations, TEST_BASE_DOMAIN.to_string());
        resend.execute(id, Uuid::new_v4()).await.unwrap();

        let sent = email.sent.lock().unwrap();
        assert!(sent[1].3.contains("https://acme.artiferris.example.com/activate?token="), "expected the acme subdomain, got: {}", sent[1].3);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn inviting_a_user_assigns_them_to_the_given_organization_and_admin_flag(pool: sqlx::PgPool) {
        let organizations = Arc::new(artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository::new(pool.clone()));
        let organization_id = Uuid::new_v4();
        organizations
            .create(&artiferris_domain::organization::Organization {
                id: organization_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        let users = Arc::new(artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool.clone()));
        let (_, invitations, hasher, email, _fake_organizations, _fake_organization_id) = setup().await;
        let use_case = InviteUserUseCase::new(users.clone(), users.clone(), invitations, hasher, email, organizations, "localhost".to_string());

        let user_id = use_case.execute(organization_id, true, "admin@acme.example", false, Uuid::new_v4()).await.unwrap();

        let created = users.find_by_id(user_id).await.unwrap().unwrap();
        assert_eq!(created.organization_id, organization_id);
        assert!(created.is_organization_admin);
        assert!(!created.is_super_admin);
    }

    fn organization(slug: &str, is_public: bool) -> Organization {
        Organization { id: Uuid::new_v4(), slug: OrganizationSlug::parse(slug).unwrap(), display_name: slug.to_string(), is_public, is_personal: false, created_at: Utc::now() }
    }

    #[test]
    fn a_dev_base_domain_ending_in_localhost_yields_http_links() {
        assert_eq!(organization_origin("artiferris.localhost", &organization("acme", false)), "http://acme.artiferris.localhost");
        assert_eq!(organization_origin("artiferris.localhost", &organization("public", true)), "http://app.artiferris.localhost");
    }

    #[test]
    fn a_real_base_domain_yields_https_links_even_if_it_starts_with_localhost() {
        assert_eq!(organization_origin("artiferris.example.com", &organization("acme", false)), "https://acme.artiferris.example.com");
        assert_eq!(organization_origin("localhost.example.com", &organization("acme", false)), "https://acme.localhost.example.com");
    }

    #[test]
    fn validate_email_rejects_embedded_control_characters() {
        assert!(validate_email("a@b.com\r\nBcc: victim@evil.com").is_err());
        assert!(validate_email("a@b.com\nX-Injected: true").is_err());
    }

    #[test]
    fn validate_email_rejects_an_address_past_the_length_limit() {
        assert!(validate_email(&format!("{}@example.com", "a".repeat(250))).is_err());
    }

    #[test]
    fn validate_email_rejects_a_second_at_sign_and_embedded_whitespace() {
        assert!(validate_email("a@b@example.com").is_err());
        assert!(validate_email("a b@example.com").is_err());
    }

    #[test]
    fn validate_email_still_accepts_ordinary_addresses() {
        assert!(validate_email("florian@example.com").is_ok());
    }
}
