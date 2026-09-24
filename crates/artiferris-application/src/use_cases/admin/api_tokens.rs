use std::sync::Arc;

use chrono::{DateTime, Utc};
use artiferris_domain::api_token::ApiTokenWithOwner;
use artiferris_domain::audit::SecurityAuditRecord;
use uuid::Uuid;

use crate::error::ApplicationError;

/// The most tokens one admin request returns.
pub const MAX_ADMIN_TOKEN_PAGE: i64 = 500;

#[derive(Debug, Clone)]
pub struct AdminApiTokenEntry {
    pub id: Uuid,
    pub user_id: Uuid,
    pub username: String,
    pub organization_id: Uuid,
    pub owner_is_super_admin: bool,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<ApiTokenWithOwner> for AdminApiTokenEntry {
    fn from(entry: ApiTokenWithOwner) -> Self {
        let ApiTokenWithOwner { token, owner_username, owner_organization_id, owner_is_super_admin } = entry;
        AdminApiTokenEntry {
            id: token.id,
            user_id: token.user_id,
            username: owner_username,
            organization_id: owner_organization_id,
            owner_is_super_admin,
            label: token.label,
            created_at: token.created_at,
            last_used_at: token.last_used_at,
            revoked_at: token.revoked_at,
        }
    }
}

pub struct AdminListApiTokensUseCase {
    tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort>,
}

impl AdminListApiTokensUseCase {
    pub fn new(tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort>) -> Self {
        Self { tokens }
    }

    /// One page, newest first. `organization_id` keeps only that organization's tokens; `limit` is cut to `MAX_ADMIN_TOKEN_PAGE`.
    pub async fn execute(&self, organization_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<AdminApiTokenEntry>, ApplicationError> {
        let entries = self.tokens.list_with_owners(organization_id, limit.clamp(1, MAX_ADMIN_TOKEN_PAGE), offset.max(0)).await?;
        Ok(entries.into_iter().map(AdminApiTokenEntry::from).collect())
    }

    pub async fn find(&self, id: Uuid) -> Result<Option<AdminApiTokenEntry>, ApplicationError> {
        Ok(self.tokens.find_with_owner(id).await?.map(AdminApiTokenEntry::from))
    }
}

pub struct AdminRevokeApiTokenUseCase {
    tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort>,
}

impl AdminRevokeApiTokenUseCase {
    pub fn new(tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort>) -> Self {
        Self { tokens }
    }

    /// `audit` goes in the same transaction.
    pub async fn execute(&self, token_id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), ApplicationError> {
        Ok(self.tokens.revoke_any(token_id, audit).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use artiferris_domain::api_token::{ApiToken, ApiTokenRepositoryPort};
    use artiferris_domain::error::DomainError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeApiTokens {
        tokens: Mutex<Vec<ApiTokenWithOwner>>,
        /// The last (organization, limit, offset) a listing was asked for.
        asked: Mutex<Option<(Option<Uuid>, i64, i64)>>,
    }

    #[async_trait]
    impl ApiTokenRepositoryPort for FakeApiTokens {
        async fn insert(&self, _token: &ApiToken) -> Result<(), DomainError> {
            unreachable!()
        }
        async fn list_for_user(&self, _user_id: Uuid) -> Result<Vec<ApiToken>, DomainError> {
            unreachable!()
        }
        async fn list_with_owners(&self, organization_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<ApiTokenWithOwner>, DomainError> {
            *self.asked.lock().unwrap() = Some((organization_id, limit, offset));
            Ok(self.tokens.lock().unwrap().iter().filter(|t| organization_id.is_none_or(|org| t.owner_organization_id == org)).cloned().collect())
        }
        async fn find_with_owner(&self, id: Uuid) -> Result<Option<ApiTokenWithOwner>, DomainError> {
            Ok(self.tokens.lock().unwrap().iter().find(|t| t.token.id == id).cloned())
        }
        async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<ApiToken>, DomainError> {
            unreachable!()
        }
        async fn touch_last_used_at(&self, _id: Uuid, _used_at: DateTime<Utc>) -> Result<(), DomainError> {
            unreachable!()
        }
        async fn revoke(&self, _id: Uuid, _user_id: Uuid) -> Result<bool, DomainError> {
            unreachable!()
        }
        async fn revoke_any(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            if let Some(t) = self.tokens.lock().unwrap().iter_mut().find(|t| t.token.id == id) {
                t.token.revoked_at = Some(Utc::now());
            }
            Ok(())
        }
    }

    fn entry(username: &str, organization_id: Uuid, label: &str) -> ApiTokenWithOwner {
        ApiTokenWithOwner {
            token: ApiToken {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                token_hash: format!("hash-{label}"),
                label: label.to_string(),
                created_at: Utc::now(),
                last_used_at: None,
                revoked_at: None,
                expires_at: None,
            },
            owner_username: username.to_string(),
            owner_organization_id: organization_id,
            owner_is_super_admin: false,
        }
    }

    #[tokio::test]
    async fn lists_tokens_with_their_owners_username_and_organization() {
        let org = Uuid::new_v4();
        let tokens = Arc::new(FakeApiTokens { tokens: Mutex::new(vec![entry("alice", org, "laptop"), entry("bob", Uuid::new_v4(), "ci")]), ..Default::default() });

        let entries = AdminListApiTokensUseCase::new(tokens).execute(None, 10, 0).await.unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].username.as_str(), entries[0].label.as_str(), entries[0].organization_id), ("alice", "laptop", org));
        assert_eq!(entries[1].username, "bob");
    }

    #[tokio::test]
    async fn the_organization_filter_reaches_the_repository_instead_of_filtering_afterwards() {
        let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
        let tokens = Arc::new(FakeApiTokens { tokens: Mutex::new(vec![entry("alice", mine, "laptop"), entry("bob", theirs, "ci")]), ..Default::default() });

        let entries = AdminListApiTokensUseCase::new(tokens.clone()).execute(Some(mine), 10, 0).await.unwrap();

        assert_eq!(entries.iter().map(|e| e.username.as_str()).collect::<Vec<_>>(), vec!["alice"]);
        assert_eq!(*tokens.asked.lock().unwrap(), Some((Some(mine), 10, 0)));
    }

    #[tokio::test]
    async fn a_page_is_never_larger_than_the_cap_and_never_negative() {
        let tokens = Arc::new(FakeApiTokens::default());
        let use_case = AdminListApiTokensUseCase::new(tokens.clone());

        use_case.execute(None, 1_000_000, -5).await.unwrap();
        assert_eq!(*tokens.asked.lock().unwrap(), Some((None, MAX_ADMIN_TOKEN_PAGE, 0)));

        use_case.execute(None, 0, 20).await.unwrap();
        assert_eq!(*tokens.asked.lock().unwrap(), Some((None, 1, 20)));
    }

    #[tokio::test]
    async fn finds_one_token_with_its_owner() {
        let org = Uuid::new_v4();
        let mut super_admins_token = entry("root", org, "ci");
        super_admins_token.owner_is_super_admin = true;
        let id = super_admins_token.token.id;
        let tokens = Arc::new(FakeApiTokens { tokens: Mutex::new(vec![super_admins_token]), ..Default::default() });

        let found = AdminListApiTokensUseCase::new(tokens).find(id).await.unwrap().unwrap();

        assert!(found.owner_is_super_admin);
        assert_eq!(found.organization_id, org);
    }

    #[tokio::test]
    async fn admin_revoke_revokes_a_token_owned_by_a_different_user() {
        let token = entry("alice", Uuid::new_v4(), "laptop");
        let token_id = token.token.id;
        let tokens = Arc::new(FakeApiTokens { tokens: Mutex::new(vec![token]), ..Default::default() });

        AdminRevokeApiTokenUseCase::new(tokens.clone()).execute(token_id, None).await.unwrap();

        assert!(tokens.tokens.lock().unwrap()[0].token.revoked_at.is_some(), "admin revoke must succeed regardless of who owns the token");
    }
}
