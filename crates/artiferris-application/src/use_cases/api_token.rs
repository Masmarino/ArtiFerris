use std::sync::Arc;

use base64::Engine;
use chrono::Utc;
use artiferris_domain::api_token::{ApiToken, ApiTokenRepositoryPort};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApplicationError;

pub fn generate_api_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("hgr_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub fn hash_api_token(plaintext: &str) -> String {
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

/// For a caller who just re-entered their password.
const REAUTHENTICATED_API_TOKEN_TTL: chrono::Duration = chrono::Duration::days(365);
/// For a caller holding only a session, which a stolen or leftover browser tab also is.
const SESSION_ONLY_API_TOKEN_TTL: chrono::Duration = chrono::Duration::days(7);
pub const MAX_ACTIVE_API_TOKENS_PER_USER: usize = 50;
pub const MAX_API_TOKEN_LABEL_LEN: usize = 100;

pub struct CreateApiTokenUseCase {
    tokens: Arc<dyn ApiTokenRepositoryPort>,
}

impl CreateApiTokenUseCase {
    pub fn new(tokens: Arc<dyn ApiTokenRepositoryPort>) -> Self {
        Self { tokens }
    }

    /// A 7-day token for a caller authenticated by a session alone. Returns `(token_id, plaintext_token)`; the
    /// plaintext is only available here.
    pub async fn execute(&self, user_id: Uuid, label: &str) -> Result<(Uuid, String), ApplicationError> {
        self.create(user_id, label, SESSION_ONLY_API_TOKEN_TTL).await
    }

    /// A 365-day token. Only for a caller that has just checked the user's password (`ConfirmPasswordUseCase`).
    pub async fn execute_reauthenticated(&self, user_id: Uuid, label: &str) -> Result<(Uuid, String), ApplicationError> {
        self.create(user_id, label, REAUTHENTICATED_API_TOKEN_TTL).await
    }

    async fn create(&self, user_id: Uuid, label: &str, ttl: chrono::Duration) -> Result<(Uuid, String), ApplicationError> {
        if label.chars().count() > MAX_API_TOKEN_LABEL_LEN {
            return Err(ApplicationError::ApiTokenLabelTooLong);
        }
        if self.tokens.list_for_user(user_id).await?.iter().filter(|t| t.is_active()).count() >= MAX_ACTIVE_API_TOKENS_PER_USER {
            return Err(ApplicationError::ApiTokenLimitReached);
        }
        let plaintext = generate_api_token();
        let now = Utc::now();
        let token = ApiToken {
            id: Uuid::new_v4(),
            user_id,
            token_hash: hash_api_token(&plaintext),
            label: label.to_string(),
            created_at: now,
            last_used_at: None,
            revoked_at: None,
            expires_at: Some(now + ttl),
        };
        self.tokens.insert(&token).await?;
        Ok((token.id, plaintext))
    }
}

pub struct ListApiTokensUseCase {
    tokens: Arc<dyn ApiTokenRepositoryPort>,
}

impl ListApiTokensUseCase {
    pub fn new(tokens: Arc<dyn ApiTokenRepositoryPort>) -> Self {
        Self { tokens }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<Vec<ApiToken>, ApplicationError> {
        Ok(self.tokens.list_for_user(user_id).await?)
    }
}

pub struct RevokeApiTokenUseCase {
    tokens: Arc<dyn ApiTokenRepositoryPort>,
}

impl RevokeApiTokenUseCase {
    pub fn new(tokens: Arc<dyn ApiTokenRepositoryPort>) -> Self {
        Self { tokens }
    }

    /// `ApiTokenNotFound` if the token does not exist or is not owned by `user_id`: zero rows affected is not a
    /// successful revoke.
    pub async fn execute(&self, token_id: Uuid, user_id: Uuid) -> Result<(), ApplicationError> {
        if self.tokens.revoke(token_id, user_id).await? {
            Ok(())
        } else {
            Err(ApplicationError::ApiTokenNotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeTokens {
        tokens: Mutex<HashMap<Uuid, artiferris_domain::api_token::ApiToken>>,
    }
    impl FakeTokens {
        fn new() -> Self { Self { tokens: Mutex::new(HashMap::new()) } }
    }
    #[async_trait::async_trait]
    impl artiferris_domain::api_token::ApiTokenRepositoryPort for FakeTokens {
        async fn insert(&self, token: &artiferris_domain::api_token::ApiToken) -> Result<(), artiferris_domain::error::DomainError> {
            self.tokens.lock().unwrap().insert(token.id, token.clone());
            Ok(())
        }
        async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<artiferris_domain::api_token::ApiToken>, artiferris_domain::error::DomainError> {
            Ok(self.tokens.lock().unwrap().values().filter(|t| t.user_id == user_id).cloned().collect())
        }
        async fn list_with_owners(&self, _organization_id: Option<Uuid>, _limit: i64, _offset: i64) -> Result<Vec<artiferris_domain::api_token::ApiTokenWithOwner>, artiferris_domain::error::DomainError> {
            unreachable!("not exercised by this use case's tests")
        }
        async fn find_with_owner(&self, _id: Uuid) -> Result<Option<artiferris_domain::api_token::ApiTokenWithOwner>, artiferris_domain::error::DomainError> {
            unreachable!("not exercised by this use case's tests")
        }
        async fn find_by_hash(&self, token_hash: &str) -> Result<Option<artiferris_domain::api_token::ApiToken>, artiferris_domain::error::DomainError> {
            Ok(self.tokens.lock().unwrap().values().find(|t| t.token_hash == token_hash).cloned())
        }
        async fn touch_last_used_at(&self, id: Uuid, used_at: chrono::DateTime<chrono::Utc>) -> Result<(), artiferris_domain::error::DomainError> {
            if let Some(t) = self.tokens.lock().unwrap().get_mut(&id) { t.last_used_at = Some(used_at); }
            Ok(())
        }
        async fn revoke(&self, id: Uuid, user_id: Uuid) -> Result<bool, artiferris_domain::error::DomainError> {
            if let Some(t) = self.tokens.lock().unwrap().get_mut(&id) {
                if t.user_id == user_id {
                    t.revoked_at = Some(chrono::Utc::now());
                    return Ok(true);
                }
            }
            Ok(false)
        }
        async fn revoke_any(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), artiferris_domain::error::DomainError> {
            if let Some(t) = self.tokens.lock().unwrap().get_mut(&id) {
                t.revoked_at = Some(chrono::Utc::now());
            }
            Ok(())
        }
    }

    #[test]
    fn generated_tokens_have_the_hgr_prefix_and_are_reasonably_long() {
        let token = generate_api_token();
        assert!(token.starts_with("hgr_"));
        assert!(token.len() > 20);
    }

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(hash_api_token("same-input"), hash_api_token("same-input"));
        assert_ne!(hash_api_token("a"), hash_api_token("b"));
    }

    #[tokio::test]
    async fn creating_a_token_stores_only_its_hash() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());
        let user_id = Uuid::new_v4();
        let (id, plaintext) = use_case.execute(user_id, "my laptop").await.unwrap();

        let stored = tokens.list_for_user(user_id).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, id);
        assert_ne!(stored[0].token_hash, plaintext, "the stored value must be a hash, never the plaintext token");
        assert_eq!(stored[0].token_hash, hash_api_token(&plaintext));
    }

    /// Every new token gets a real, future expiry by default.
    #[tokio::test]
    async fn a_newly_created_token_has_a_future_expiry() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());
        let user_id = Uuid::new_v4();
        let (id, _) = use_case.execute(user_id, "my laptop").await.unwrap();

        let stored = tokens.list_for_user(user_id).await.unwrap();
        let token = stored.iter().find(|t| t.id == id).unwrap();
        assert!(token.expires_at.is_some(), "a newly created token must have an expiry set");
        assert!(token.expires_at.unwrap() > Utc::now(), "a freshly created token's expiry must be in the future");
    }

    #[tokio::test]
    async fn revoking_someone_elses_token_is_rejected_and_does_not_revoke_it() {
        let tokens = Arc::new(FakeTokens::new());
        let create = CreateApiTokenUseCase::new(tokens.clone());
        let owner = Uuid::new_v4();
        let (id, _) = create.execute(owner, "laptop").await.unwrap();

        let revoke = RevokeApiTokenUseCase::new(tokens.clone());
        let err = revoke.execute(id, Uuid::new_v4()).await.unwrap_err(); // different user_id
        assert!(matches!(err, ApplicationError::ApiTokenNotFound));

        let stored = tokens.list_for_user(owner).await.unwrap();
        assert!(stored[0].revoked_at.is_none(), "revoke must not affect a token owned by a different user");
    }

    #[tokio::test]
    async fn revoking_an_unknown_token_id_is_rejected() {
        let tokens = Arc::new(FakeTokens::new());
        let revoke = RevokeApiTokenUseCase::new(tokens.clone());
        let err = revoke.execute(Uuid::new_v4(), Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::ApiTokenNotFound));
    }

    #[tokio::test]
    async fn a_token_created_without_reauthenticating_lives_at_most_7_days() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());
        let user_id = Uuid::new_v4();

        use_case.execute(user_id, "laptop").await.unwrap();

        let expires_at = tokens.list_for_user(user_id).await.unwrap()[0].expires_at.unwrap();
        assert!(expires_at <= Utc::now() + chrono::Duration::days(7) + chrono::Duration::minutes(1));
        assert!(expires_at > Utc::now() + chrono::Duration::days(6));
    }

    #[tokio::test]
    async fn a_reauthenticated_token_lives_365_days() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());
        let user_id = Uuid::new_v4();

        use_case.execute_reauthenticated(user_id, "ci").await.unwrap();

        let expires_at = tokens.list_for_user(user_id).await.unwrap()[0].expires_at.unwrap();
        assert!(expires_at > Utc::now() + chrono::Duration::days(364));
    }

    #[tokio::test]
    async fn an_overlong_label_is_rejected() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());

        let err = use_case.execute(Uuid::new_v4(), &"x".repeat(MAX_API_TOKEN_LABEL_LEN + 1)).await.unwrap_err();

        assert!(matches!(err, ApplicationError::ApiTokenLabelTooLong));
    }

    #[tokio::test]
    async fn a_user_cannot_hold_more_than_the_maximum_number_of_active_tokens() {
        let tokens = Arc::new(FakeTokens::new());
        let use_case = CreateApiTokenUseCase::new(tokens.clone());
        let user_id = Uuid::new_v4();
        let mut first_id = None;
        for i in 0..MAX_ACTIVE_API_TOKENS_PER_USER {
            let (id, _) = use_case.execute(user_id, &format!("token {i}")).await.unwrap();
            first_id.get_or_insert(id);
        }

        let err = use_case.execute(user_id, "one too many").await.unwrap_err();
        assert!(matches!(err, ApplicationError::ApiTokenLimitReached));

        RevokeApiTokenUseCase::new(tokens.clone()).execute(first_id.unwrap(), user_id).await.unwrap();
        use_case.execute(user_id, "fits again").await.unwrap();
    }
}
