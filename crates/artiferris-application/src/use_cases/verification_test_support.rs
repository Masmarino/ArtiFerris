//! A `UserSecurityPort` that only knows which accounts hold a verified address, for tests of notices that must reach verified addresses only.

use async_trait::async_trait;
use artiferris_domain::audit::SecurityAuditRecord;
use artiferris_domain::error::DomainError;
use artiferris_domain::user::{User, UserSecurityPort};
use uuid::Uuid;

pub struct FakeVerification {
    verified: Vec<User>,
}

impl FakeVerification {
    pub fn nobody() -> Self {
        Self { verified: Vec::new() }
    }

    pub fn of(users: &[&User]) -> Self {
        Self { verified: users.iter().map(|user| (*user).clone()).collect() }
    }
}

#[async_trait]
impl UserSecurityPort for FakeVerification {
    async fn revoke_sessions(&self, _id: Uuid, _audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        unreachable!("not exercised by these tests")
    }
    async fn find_by_verified_email(&self, organization_id: Uuid, email: &str) -> Result<Option<User>, DomainError> {
        Ok(self.verified.iter().find(|u| u.organization_id == organization_id && u.email.as_deref().is_some_and(|own| own.eq_ignore_ascii_case(email))).cloned())
    }
    async fn insert_with_verified_email(&self, _user: &User) -> Result<(), DomainError> {
        unreachable!("not exercised by these tests")
    }
    async fn mark_email_verified(&self, _id: Uuid) -> Result<bool, DomainError> {
        unreachable!("not exercised by these tests")
    }
}
