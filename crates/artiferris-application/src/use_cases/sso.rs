use std::sync::Arc;

use artiferris_domain::error::DomainError;
use artiferris_domain::reserved_names::is_reserved_name;
use artiferris_domain::sso::ExternalIdentity;
use artiferris_domain::user::{PasswordHasherPort, TokenIssuerPort, User, UserRepositoryPort, UserSecurityPort, Username};
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::invitation::{unusable_password_hash, validate_email};

fn derive_username_candidate(email: &str) -> String {
    let local_part = email.split('@').next().unwrap_or(email).to_lowercase();
    let filtered: String = local_part.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    let starts_with_letter = filtered.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let mut candidate = if starts_with_letter { filtered } else { format!("u{filtered}") };
    if candidate.len() < 3 {
        candidate.push_str("-user");
    }
    if is_reserved_name(&candidate) {
        candidate = format!("user-{candidate}");
    }
    candidate.chars().take(32).collect()
}

pub struct ProvisionSsoUserUseCase {
    users: Arc<dyn UserRepositoryPort>,
    security: Arc<dyn UserSecurityPort>,
    hasher: Arc<dyn PasswordHasherPort>,
    tokens: Arc<dyn TokenIssuerPort>,
    system_settings: Arc<dyn artiferris_domain::system_settings::SystemSettingsPort>,
}

impl ProvisionSsoUserUseCase {
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        security: Arc<dyn UserSecurityPort>,
        hasher: Arc<dyn PasswordHasherPort>,
        tokens: Arc<dyn TokenIssuerPort>,
        system_settings: Arc<dyn artiferris_domain::system_settings::SystemSettingsPort>,
    ) -> Self {
        Self { users, security, hasher, tokens, system_settings }
    }

    pub async fn execute(&self, organization_id: Uuid, identity: &ExternalIdentity) -> Result<String, ApplicationError> {
        // Accounts are linked by this address, so a placeholder or malformed one is refused rather than shared between people.
        let email = identity.email.trim().to_lowercase();
        if let Err(e) = validate_email(&email) {
            tracing::warn!("refusing an SSO login whose identity provider sent an unusable email: {e}");
            return Err(ApplicationError::InvalidCredentials);
        }
        let identity = &ExternalIdentity { email, ..identity.clone() };
        if let Some(token) = self.sign_in_linked_account(organization_id, identity).await? {
            return Ok(token);
        }

        let base = derive_username_candidate(&identity.email);
        let mut candidate = base.clone();
        let mut suffix = 1u32;
        loop {
            let parsed = Username::parse_new(&candidate)?;
            if self.users.find_by_username(&parsed).await?.is_none() {
                let user = User {
                    id: Uuid::new_v4(),
                    username: parsed,
                    password_hash: unusable_password_hash(self.hasher.as_ref()).await?,
                    is_super_admin: false,
                    is_organization_admin: false,
                    organization_id,
                    created_at: chrono::Utc::now(),
                    tokens_valid_after: chrono::Utc::now(),
                    email: Some(identity.email.clone()),
                };
                match self.security.insert_with_verified_email(&user).await {
                    Ok(()) => {
                        let settings = self.system_settings.get(organization_id).await?;
                        return Ok(self.tokens.issue(user.id, chrono::Duration::hours(settings.session_ttl_hours as i64))?);
                    }
                    // A parallel first login for the same person got there first: link to what it created.
                    Err(taken @ (DomainError::EmailTaken | DomainError::UsernameTaken)) => {
                        if let Some(token) = self.sign_in_linked_account(organization_id, identity).await? {
                            return Ok(token);
                        }
                        if taken == DomainError::EmailTaken {
                            return Err(taken.into());
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            suffix += 1;
            let suffix_str = format!("-{suffix}");
            let truncated_base: String = base.chars().take(32 - suffix_str.len()).collect();
            candidate = format!("{truncated_base}{suffix_str}");
        }
    }

    /// Links only a verified account of this organization, never an admin: a directory attribute like `mail` must not hand out an admin session.
    async fn sign_in_linked_account(&self, organization_id: Uuid, identity: &ExternalIdentity) -> Result<Option<String>, ApplicationError> {
        let Some(existing) = self.security.find_by_verified_email(organization_id, &identity.email).await? else {
            return Ok(None);
        };
        if existing.is_super_admin || existing.is_organization_admin {
            return Err(ApplicationError::InvalidCredentials);
        }
        let settings = self.system_settings.get(organization_id).await?;
        Ok(Some(self.tokens.issue(existing.id, chrono::Duration::hours(settings.session_ttl_hours as i64))?))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::organization::OrganizationRepositoryPort;

    use super::*;

    struct FakeUsers {
        users: Mutex<HashMap<Uuid, User>>,
        verified: Mutex<HashSet<Uuid>>,
    }

    impl FakeUsers {
        fn new() -> Self {
            Self { users: Mutex::new(HashMap::new()), verified: Mutex::new(HashSet::new()) }
        }

        fn seeded(users: Vec<User>) -> Self {
            let verified = users.iter().map(|u| u.id).collect();
            Self { users: Mutex::new(users.into_iter().map(|u| (u.id, u)).collect()), verified: Mutex::new(verified) }
        }

        fn seeded_unverified(users: Vec<User>) -> Self {
            Self { users: Mutex::new(users.into_iter().map(|u| (u.id, u)).collect()), verified: Mutex::new(HashSet::new()) }
        }
    }

    #[async_trait]
    impl UserSecurityPort for FakeUsers {
        async fn revoke_sessions(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> {
            if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
                user.tokens_valid_after = chrono::Utc::now();
            }
            Ok(())
        }
        async fn find_by_verified_email(&self, organization_id: Uuid, email: &str) -> Result<Option<User>, DomainError> {
            let verified = self.verified.lock().unwrap();
            Ok(self.users.lock().unwrap().values().find(|u| u.organization_id == organization_id && verified.contains(&u.id) && u.email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(email))).cloned())
        }
        async fn insert_with_verified_email(&self, user: &User) -> Result<(), DomainError> {
            self.users.lock().unwrap().insert(user.id, user.clone());
            self.verified.lock().unwrap().insert(user.id);
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

    struct FakeTokenIssuer;

    #[async_trait]
    impl TokenIssuerPort for FakeTokenIssuer {
        fn issue(&self, user_id: Uuid, _ttl: chrono::Duration) -> Result<String, DomainError> {
            Ok(format!("token:{user_id}"))
        }
        fn verify(&self, token: &str) -> Result<artiferris_domain::user::VerifiedToken, DomainError> {
            let user_id = token.strip_prefix("token:").and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| DomainError::InvalidUsername("bad token".to_string()))?;
            Ok(artiferris_domain::user::VerifiedToken { user_id, issued_at: chrono::Utc::now() })
        }
    }

    struct FakeSystemSettings;

    #[async_trait]
    impl artiferris_domain::system_settings::SystemSettingsPort for FakeSystemSettings {
        async fn get(&self, _organization_id: Uuid) -> Result<artiferris_domain::system_settings::SystemSettings, DomainError> {
            Ok(artiferris_domain::system_settings::SystemSettings::defaults())
        }
        async fn update(&self, _organization_id: Uuid, _settings: &artiferris_domain::system_settings::SystemSettings, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn provisions_a_fresh_member_account_on_first_login() {
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users, Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let organization_id = Uuid::new_v4();
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: Some("Florian".to_string()) };

        let token = use_case.execute(organization_id, &identity).await.unwrap();

        assert!(token.starts_with("token:"));
    }

    #[tokio::test]
    async fn an_email_the_provider_sends_in_odd_case_or_with_padding_is_normalised_before_it_becomes_the_key() {
        let organization_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));

        let first = use_case.execute(organization_id, &ExternalIdentity { email: "  Florian@Corp.Example ".to_string(), display_name: None }).await.unwrap();
        let second = use_case.execute(organization_id, &ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None }).await.unwrap();

        assert_eq!(first, second, "both spellings are the same person");
        let stored = users.find_by_verified_email(organization_id, "florian@corp.example").await.unwrap().unwrap();
        assert_eq!(stored.email.as_deref(), Some("florian@corp.example"));
    }

    #[tokio::test]
    async fn an_email_that_cannot_be_an_address_never_provisions_or_links_anything() {
        let organization_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let too_long = format!("{}@corp.example", "a".repeat(250));

        for bad in ["", "   ", "no-at-sign", "@corp.example", "a@", "a@localhost", "a@b@corp.example", "a b@corp.example", "a@corp.example\r\nBcc: x@evil.example", too_long.as_str()] {
            let outcome = use_case.execute(organization_id, &ExternalIdentity { email: bad.to_string(), display_name: None }).await;
            assert!(matches!(outcome, Err(ApplicationError::InvalidCredentials)), "{bad:?} got {outcome:?}");
        }
        assert!(users.list_all().await.unwrap().is_empty(), "nothing was provisioned for any of them");
    }

    #[tokio::test]
    async fn a_provisioned_account_is_never_an_admin_regardless_of_display_name_content() {
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let organization_id = Uuid::new_v4();
        // A directory attribute an attacker fully controls must never influence the role.
        let identity = ExternalIdentity { email: "attacker@corp.example".to_string(), display_name: Some("Admin Super-Admin Root".to_string()) };

        use_case.execute(organization_id, &identity).await.unwrap();

        let created = users.users.lock().unwrap().values().find(|u| u.email.as_deref() == Some("attacker@corp.example")).cloned().unwrap();
        assert!(!created.is_super_admin);
        assert!(!created.is_organization_admin);
    }

    #[tokio::test]
    async fn a_provisioned_account_has_no_usable_password() {
        let users = Arc::new(FakeUsers::new());
        let hasher = Arc::new(FakeHasher);
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), hasher.clone(), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        use_case.execute(Uuid::new_v4(), &identity).await.unwrap();

        let created = users.users.lock().unwrap().values().next().cloned().unwrap();
        assert!(!hasher.verify("anything", &created.password_hash).await.unwrap(), "no plaintext should ever verify against a JIT-provisioned account's placeholder hash");
    }

    #[tokio::test]
    async fn logging_in_again_with_the_same_email_reuses_the_existing_account() {
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };
        let organization_id = Uuid::new_v4();

        use_case.execute(organization_id, &identity).await.unwrap();
        let first_count = users.users.lock().unwrap().len();
        use_case.execute(organization_id, &identity).await.unwrap();
        let second_count = users.users.lock().unwrap().len();

        assert_eq!(first_count, second_count, "a second login with the same email must not create a second account");
    }

    #[test]
    fn a_reserved_email_local_part_gets_a_usable_username() {
        let candidate = derive_username_candidate("artiferris-npm@corp.example");
        assert!(!is_reserved_name(&candidate));
        assert!(Username::parse_new(&candidate).is_ok());
    }

    #[tokio::test]
    async fn provisioning_an_account_whose_email_starts_with_the_reserved_prefix_succeeds() {
        let organization_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "artiferris-docker@corp.example".to_string(), display_name: None };

        use_case.execute(organization_id, &identity).await.unwrap();

        let created = users.users.lock().unwrap().values().next().unwrap().username.as_str().to_string();
        assert!(!is_reserved_name(&created), "{created} must not carry the reserved prefix");
    }

    #[tokio::test]
    async fn an_account_of_a_different_organization_is_never_linked() {
        let organization_a = Uuid::new_v4();
        let organization_b = Uuid::new_v4();
        let existing = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "placeholder".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: organization_a,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("florian@corp.example".to_string()),
        };
        let existing_id = existing.id;
        let users = Arc::new(FakeUsers::seeded(vec![existing]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let token = use_case.execute(organization_b, &identity).await.unwrap();

        assert_ne!(token, format!("token:{existing_id}"), "the other organization's account must not be signed in to");
        let users = users.users.lock().unwrap();
        assert_eq!(users.len(), 2);
        assert!(users.values().any(|u| u.organization_id == organization_b && u.email.as_deref() == Some("florian@corp.example")), "a fresh account is provisioned in the organization the login came in on");
    }

    #[tokio::test]
    async fn reusing_an_existing_super_admin_account_is_rejected_even_in_the_same_organization() {
        let organization_id = Uuid::new_v4();
        let existing = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "placeholder".to_string(),
            is_super_admin: true,
            is_organization_admin: false,
            organization_id,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("florian@corp.example".to_string()),
        };
        let users = Arc::new(FakeUsers::seeded(vec![existing]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let err = use_case.execute(organization_id, &identity).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidCredentials), "got {err:?}");
    }

    #[tokio::test]
    async fn reusing_an_existing_organization_admin_account_is_rejected_even_in_the_same_organization() {
        let organization_id = Uuid::new_v4();
        let existing = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "placeholder".to_string(),
            is_super_admin: false,
            is_organization_admin: true,
            organization_id,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("florian@corp.example".to_string()),
        };
        let users = Arc::new(FakeUsers::seeded(vec![existing]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let err = use_case.execute(organization_id, &identity).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidCredentials), "got {err:?}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_account_of_a_different_organization_is_never_linked_against_a_real_database(pool: sqlx::PgPool) {
        let organizations = Arc::new(artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository::new(pool.clone()));
        let organization_a = Uuid::new_v4();
        let organization_b = Uuid::new_v4();
        organizations
            .create(&artiferris_domain::organization::Organization {
                id: organization_a,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        organizations
            .create(&artiferris_domain::organization::Organization {
                id: organization_b,
                slug: artiferris_domain::organization::OrganizationSlug::parse("globex").unwrap(),
                display_name: "Globex".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let users = Arc::new(artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool.clone()));
        let super_admin_id = Uuid::new_v4();
        users
            .insert_with_verified_email(&User {
                id: super_admin_id,
                username: Username::parse("acme-super-admin").unwrap(),
                password_hash: "placeholder".to_string(),
                is_super_admin: true,
                is_organization_admin: false,
                organization_id: organization_a,
                created_at: chrono::Utc::now(),
                tokens_valid_after: chrono::Utc::now(),
                email: Some("root@acme.example".to_string()),
            })
            .await
            .unwrap();

        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "root@acme.example".to_string(), display_name: None };

        let token = use_case.execute(organization_b, &identity).await.unwrap();

        assert_ne!(token, format!("token:{super_admin_id}"), "SSO in another organization must never sign in to the super-admin");
        let provisioned = users.find_by_verified_email(organization_b, "root@acme.example").await.unwrap().unwrap();
        assert_ne!(provisioned.id, super_admin_id);
        assert!(!provisioned.is_super_admin && !provisioned.is_organization_admin);
        assert_eq!(users.find_by_verified_email(organization_a, "root@acme.example").await.unwrap().unwrap().id, super_admin_id, "the super-admin's own verified email is untouched");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn parallel_first_logins_of_one_person_end_up_on_one_account(pool: sqlx::PgPool) {
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let users = Arc::new(artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool));
        let use_case = Arc::new(ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings)));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let use_case = use_case.clone();
                let identity = identity.clone();
                tokio::spawn(async move { use_case.execute(organization_id, &identity).await })
            })
            .collect();
        let tokens: Vec<String> = futures::future::join_all(handles).await.into_iter().map(|r| r.unwrap().expect("no parallel login may fail")).collect();

        assert!(tokens.iter().all(|t| t == &tokens[0]), "every login signs in to the one account: {tokens:?}");
        assert_eq!(users.count_by_organization(organization_id).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_organizations_provision_separate_accounts_for_the_same_verified_email(pool: sqlx::PgPool) {
        let organizations = artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository::new(pool.clone());
        let acme = Uuid::new_v4();
        organizations
            .create(&artiferris_domain::organization::Organization {
                id: acme,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let public = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let users = Arc::new(artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let in_public = use_case.execute(public, &identity).await.unwrap();
        let in_acme = use_case.execute(acme, &identity).await.unwrap();

        assert_ne!(in_public, in_acme);
        assert_eq!(use_case.execute(public, &identity).await.unwrap(), in_public, "each organization keeps signing in to its own account");
        assert_eq!(use_case.execute(acme, &identity).await.unwrap(), in_acme);
    }

    #[tokio::test]
    async fn derives_a_username_from_the_email_local_part() {
        let users = Arc::new(FakeUsers::new());
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian.dupont@corp.example".to_string(), display_name: None };

        use_case.execute(Uuid::new_v4(), &identity).await.unwrap();

        let created = users.users.lock().unwrap().values().next().cloned().unwrap();
        assert_eq!(created.username.as_str(), "floriandupont");
    }

    #[tokio::test]
    async fn a_colliding_username_gets_a_numeric_suffix() {
        let existing = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "placeholder".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::new_v4(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("someone-else@corp.example".to_string()),
        };
        let users = Arc::new(FakeUsers::seeded(vec![existing]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        use_case.execute(Uuid::new_v4(), &identity).await.unwrap();

        let created = users.users.lock().unwrap().values().find(|u| u.email.as_deref() == Some("florian@corp.example")).cloned().unwrap();
        assert_eq!(created.username.as_str(), "florian-2");
    }

    fn member_with_email(organization_id: Uuid, username: &str, email: &str) -> User {
        User {
            id: Uuid::new_v4(),
            username: Username::parse(username).unwrap(),
            password_hash: "attacker-chosen-hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some(email.to_string()),
        }
    }

    #[tokio::test]
    async fn an_unverified_self_registered_account_is_not_linked_by_email() {
        let organization_id = Uuid::new_v4();
        let squatter = member_with_email(organization_id, "squatter", "florian@corp.example");
        let squatter_id = squatter.id;
        let users = Arc::new(FakeUsers::seeded_unverified(vec![squatter]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let token = use_case.execute(organization_id, &identity).await.unwrap();

        assert_ne!(token, format!("token:{squatter_id}"), "the session must not be for the account the attacker registered");
        assert_eq!(users.users.lock().unwrap().len(), 2, "a fresh account is provisioned next to the unverified one");
    }

    #[tokio::test]
    async fn a_verified_account_is_linked_by_email_regardless_of_case() {
        let organization_id = Uuid::new_v4();
        let invited = member_with_email(organization_id, "florian", "Florian@Corp.Example");
        let invited_id = invited.id;
        let users = Arc::new(FakeUsers::seeded(vec![invited]));
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        let token = use_case.execute(organization_id, &identity).await.unwrap();

        assert_eq!(token, format!("token:{invited_id}"));
    }

    #[tokio::test]
    async fn a_provisioned_account_counts_as_verified_for_its_next_login() {
        let users = Arc::new(FakeUsers::new());
        let organization_id = Uuid::new_v4();
        let use_case = ProvisionSsoUserUseCase::new(users.clone(), users.clone(), Arc::new(FakeHasher), Arc::new(FakeTokenIssuer), Arc::new(FakeSystemSettings));
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None };

        use_case.execute(organization_id, &identity).await.unwrap();

        assert!(users.find_by_verified_email(organization_id, "florian@corp.example").await.unwrap().is_some());
    }
}
