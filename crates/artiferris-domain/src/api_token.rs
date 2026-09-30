use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::audit::SecurityAuditRecord;
use crate::error::DomainError;

#[derive(Debug, Clone)]
pub struct ApiToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// `None` means no expiry: only rows that predate expiry. New tokens always get one.
    pub expires_at: Option<DateTime<Utc>>,
}

impl ApiToken {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|exp| exp > Utc::now())
    }
}

#[derive(Debug, Clone)]
pub struct ApiTokenWithOwner {
    pub token: ApiToken,
    pub owner_username: String,
    pub owner_organization_id: Uuid,
    pub owner_is_super_admin: bool,
}

#[async_trait]
pub trait ApiTokenRepositoryPort: Send + Sync {
    async fn insert(&self, token: &ApiToken) -> Result<(), DomainError>;
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<ApiToken>, DomainError>;
    /// One page of tokens, newest first, revoked included; `organization_id` keeps only tokens whose owner belongs to
    /// it.
    async fn list_with_owners(&self, organization_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<ApiTokenWithOwner>, DomainError>;
    async fn find_with_owner(&self, id: Uuid) -> Result<Option<ApiTokenWithOwner>, DomainError>;
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>, DomainError>;
    async fn touch_last_used_at(&self, id: Uuid, used_at: DateTime<Utc>) -> Result<(), DomainError>;
    /// `true` if a token owned by `user_id` was revoked, `false` if no row matched: callers must not treat that as
    /// success.
    async fn revoke(&self, id: Uuid, user_id: Uuid) -> Result<bool, DomainError>;
    /// Admin override: revokes whatever the owner. `audit` goes in the same transaction.
    async fn revoke_any(&self, id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_without_a_revocation_date_is_active() {
        let token = ApiToken {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            token_hash: "hash".to_string(),
            label: "my laptop".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
            expires_at: None,
        };
        assert!(token.is_active());
    }

    #[test]
    fn a_revoked_token_is_not_active() {
        let token = ApiToken {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            token_hash: "hash".to_string(),
            label: "my laptop".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: Some(Utc::now()),
            expires_at: None,
        };
        assert!(!token.is_active());
    }

    #[test]
    fn a_token_with_a_future_expiry_is_active() {
        let token = ApiToken {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            token_hash: "hash".to_string(),
            label: "my laptop".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
            expires_at: Some(Utc::now() + chrono::Duration::days(1)),
        };
        assert!(token.is_active());
    }

    #[test]
    fn a_token_with_a_past_expiry_is_not_active() {
        let token = ApiToken {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            token_hash: "hash".to_string(),
            label: "my laptop".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
            expires_at: Some(Utc::now() - chrono::Duration::days(1)),
        };
        assert!(!token.is_active());
    }
}
