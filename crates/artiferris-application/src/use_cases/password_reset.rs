//! A password reset by an administrator: they issue a link valid for one hour, the person follows it and chooses a new
//! password. Tokens work like invitations: 64 random hex characters, only their SHA-256 stored, used once.

use std::sync::Arc;

use artiferris_domain::audit::{AdminAuditEvent, AdminAuditRecord, AuditRecord, SecurityAuditRecord, SecurityEvent};
use artiferris_domain::email::EmailPort;
use artiferris_domain::error::DomainError;
use artiferris_domain::invitation::UserInvitationPort;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::password_reset::{PasswordReset, PasswordResetPort};
use artiferris_domain::sso::IdentityProviderRepositoryPort;
use artiferris_domain::user::{Password, PasswordHasherPort, UserRepositoryPort, UserSecurityPort};
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::invitation::{MailFailure, generate_invitation_token, hash_invitation_token, organization_origin, require_organization, unusable_password_hash};
use crate::use_cases::mfa::verified_address;

pub(crate) const PASSWORD_RESET_TTL_HOURS: i64 = 1;

/// A reset whose mail did not go out: the link it carried, for the administrator to pass on, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndeliveredReset {
    pub reset_url: String,
    pub reason: MailFailure,
}

/// Replaces any previous link. The current password is swapped for an unusable hash at once, and every session ends
/// with it: otherwise a link left unused would leave the old password working.
pub struct AdminResetPasswordUseCase {
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    invitations: Arc<dyn UserInvitationPort>,
    identity_providers: Arc<dyn IdentityProviderRepositoryPort>,
    resets: Arc<dyn PasswordResetPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    email: Arc<dyn EmailPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    artiferris_base_domain: String,
}

impl AdminResetPasswordUseCase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        security: Arc<dyn UserSecurityPort>,
        invitations: Arc<dyn UserInvitationPort>,
        identity_providers: Arc<dyn IdentityProviderRepositoryPort>,
        resets: Arc<dyn PasswordResetPort>,
        hasher: Arc<dyn PasswordHasherPort>,
        email: Arc<dyn EmailPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
        artiferris_base_domain: String,
    ) -> Self {
        Self { users, security, invitations, identity_providers, resets, hasher, email, organizations, artiferris_base_domain }
    }

    /// The caller has checked the target exists and is within the actor's reach. The link goes to the account's
    /// verified address; when it cannot, it comes back for the administrator to pass on. The `PasswordReset` audit
    /// entry is written with the voided password.
    pub async fn execute(&self, target_id: Uuid, actor_id: Uuid) -> Result<Option<UndeliveredReset>, ApplicationError> {
        if target_id == actor_id {
            return Err(ApplicationError::OwnPasswordReset);
        }
        let user = self.users.find_by_id(target_id).await?.ok_or(ApplicationError::Domain(DomainError::Infrastructure(format!("user {target_id} not found"))))?;
        // An invited account already has its activation link: a reset link on top would make two.
        if self.invitations.find_by_user_id(user.id).await?.is_some() {
            return Err(ApplicationError::AccountNotActivated);
        }
        if self.identity_providers.get(user.organization_id).await?.is_some() {
            return Err(ApplicationError::PasswordManagedByIdentityProvider);
        }
        let organization = require_organization(self.organizations.as_ref(), user.organization_id).await?;

        // Hashing is slow and may fail: before anything is written.
        let unusable = unusable_password_hash(self.hasher.as_ref()).await?;
        let audit = AuditRecord::Admin(AdminAuditRecord { event: AdminAuditEvent::PasswordReset { user_id: user.id, organization_id: user.organization_id }, actor_id: Some(actor_id) });
        // Voids the password and ends every session; if storing the link then fails, the account is locked with no
        // link and the administrator tries again.
        self.users.update_password(user.id, unusable, Some(&audit)).await?;
        let token = generate_invitation_token();
        self.resets.replace(&PasswordReset { user_id: user.id, token_hash: hash_invitation_token(&token), expires_at: Utc::now() + Duration::hours(PASSWORD_RESET_TTL_HOURS) }).await?;

        let reset_url = format!("{}/reset-password#token={token}", organization_origin(&self.artiferris_base_domain, &organization));
        let Some(address) = verified_address(self.security.as_ref(), &user).await else {
            return Ok(Some(UndeliveredReset { reset_url, reason: MailFailure::NoAddress }));
        };
        let language = self.email.language_for(user.id).await;
        let content = crate::email_templates::password_reset(language, user.username.as_str(), &reset_url);
        match self.email.send(user.organization_id, &address, &content.subject, &content.text, &content.html).await {
            Ok(()) => Ok(None),
            Err(e) => {
                tracing::warn!("failed to send the password-reset email of user {}: {e}", user.id);
                let reason = if matches!(e, DomainError::EmailNotConfigured) { MailFailure::NotConfigured } else { MailFailure::SendFailed };
                Ok(Some(UndeliveredReset { reset_url, reason }))
            }
        }
    }
}

/// No session is issued: the person signs in afterwards, second factor included, which a reset leaves alone.
pub struct ConsumePasswordResetUseCase {
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    resets: Arc<dyn PasswordResetPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    email: Arc<dyn EmailPort>,
}

impl ConsumePasswordResetUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, security: Arc<dyn UserSecurityPort>, resets: Arc<dyn PasswordResetPort>, hasher: Arc<dyn PasswordHasherPort>, email: Arc<dyn EmailPort>) -> Self {
        Self { users, security, resets, hasher, email }
    }

    /// The password is checked and hashed before the link is used, so a refused password does not burn it. Returns
    /// whose password was set.
    pub async fn execute(&self, token: &str, new_password: &str) -> Result<Uuid, ApplicationError> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApplicationError::PasswordResetLinkInvalid);
        }
        let password = Password::parse(new_password)?;
        let hash = self.hasher.hash(password.as_str()).await?;
        let reset = self.resets.consume(&hash_invitation_token(token)).await?.ok_or(ApplicationError::PasswordResetLinkInvalid)?;
        let user = self.users.find_by_id(reset.user_id).await?.ok_or(ApplicationError::PasswordResetLinkInvalid)?;
        let audit = AuditRecord::Security(SecurityAuditRecord { event: SecurityEvent::PasswordChanged { user_id: user.id, organization_id: user.organization_id }, actor_id: Some(user.id) });
        if let Err(e) = self.users.update_password(user.id, hash, Some(&audit)).await {
            // Put the link back so the person can try again, unless an administrator issued a newer one meanwhile.
            if let Err(restore_error) = self.resets.restore(&reset).await {
                tracing::error!("failed to restore the password-reset link of user {} after a failed write: {restore_error}", user.id);
            }
            return Err(e.into());
        }
        // A notice that does not go out does not undo a password already set.
        if let Some(address) = verified_address(self.security.as_ref(), &user).await {
            let language = self.email.language_for(user.id).await;
            let content = crate::email_templates::password_changed(language, user.username.as_str());
            if let Err(e) = self.email.send(user.organization_id, &address, &content.subject, &content.text, &content.html).await {
                tracing::warn!("failed to send the password-change notice of user {}: {e}", user.id);
            }
        }
        Ok(user.id)
    }
}
