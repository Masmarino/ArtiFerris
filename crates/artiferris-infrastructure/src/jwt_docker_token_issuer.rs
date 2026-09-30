use async_trait::async_trait;
use chrono::{Duration, Utc};
use artiferris_domain::docker_registry::{DOCKER_ACCESS_TOKEN_TTL_SECONDS, DockerAccessClaims, DockerGrantedScope, DockerTokenIssuerPort};
use artiferris_domain::error::DomainError;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error_ext::InfraErr;
use crate::token_keys::derive_signing_key;

#[derive(Debug, Serialize, Deserialize)]
struct ScopeClaim {
    resource_type: String,
    name: String,
    actions: Vec<String>,
    granted_repository_id: Option<Uuid>,
}

const DOCKER_ACCESS_TOKEN_TYPE: &str = "docker-access";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: Uuid,
    /// Lets the data-plane extractor reject a token minted before the user's `tokens_valid_after` (M-17).
    iat: i64,
    exp: i64,
    scope: Option<ScopeClaim>,
    typ: String,
    org: Uuid,
    super_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_token: Option<Uuid>,
}

pub struct JwtDockerTokenIssuer {
    key: [u8; 32],
    ttl: Duration,
}

impl JwtDockerTokenIssuer {
    pub fn new(secret: String) -> Self {
        Self::with_ttl(secret, Duration::seconds(DOCKER_ACCESS_TOKEN_TTL_SECONDS))
    }

    pub fn with_ttl(secret: String, ttl: Duration) -> Self {
        Self { key: derive_signing_key(&secret, DOCKER_ACCESS_TOKEN_TYPE), ttl }
    }

    fn mint(&self, api_token_id: Option<Uuid>, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
        let now = Utc::now();
        let claims = Claims {
            sub: user_id,
            iat: now.timestamp(),
            exp: (now + self.ttl).timestamp(),
            scope: granted_scope
                .map(|s| ScopeClaim { resource_type: s.resource_type, name: s.name, actions: s.actions, granted_repository_id: s.granted_repository_id }),
            typ: DOCKER_ACCESS_TOKEN_TYPE.to_string(),
            org: organization_id,
            super_admin: is_super_admin,
            api_token: api_token_id,
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(&self.key)).infra_err()
    }
}

#[async_trait]
impl DockerTokenIssuerPort for JwtDockerTokenIssuer {
    fn issue(&self, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
        self.mint(None, user_id, organization_id, is_super_admin, granted_scope)
    }

    fn issue_for_api_token(&self, api_token_id: Uuid, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
        self.mint(Some(api_token_id), user_id, organization_id, is_super_admin, granted_scope)
    }

    fn verify(&self, token: &str) -> Result<DockerAccessClaims, DomainError> {
        // No leeway: the library's default of 60 seconds would stretch the token's short lifetime by half again.
        let mut validation = Validation::default();
        validation.leeway = 0;
        let data = decode::<Claims>(token, &DecodingKey::from_secret(&self.key), &validation).infra_err()?;
        if data.claims.typ != DOCKER_ACCESS_TOKEN_TYPE {
            return Err(DomainError::Infrastructure("not a docker access token".to_string()));
        }
        Ok(DockerAccessClaims {
            user_id: data.claims.sub,
            organization_id: data.claims.org,
            is_super_admin: data.claims.super_admin,
            granted_scope: data.claims.scope.map(|s| DockerGrantedScope {
                resource_type: s.resource_type,
                name: s.name,
                actions: s.actions,
                granted_repository_id: s.granted_repository_id,
            }),
            issued_at: chrono::DateTime::from_timestamp(data.claims.iat, 0)
                .ok_or_else(|| DomainError::Infrastructure("invalid iat claim".to_string()))?,
            api_token_id: data.claims.api_token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuing_then_verifying_with_no_scope_round_trips_the_user_id() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let user_id = Uuid::new_v4();
        let token = issuer.issue(user_id, Uuid::new_v4(), false, None).unwrap();
        let claims = issuer.verify(&token).unwrap();
        assert_eq!(claims.user_id, user_id);
        assert!(claims.granted_scope.is_none());
    }

    /// `issued_at` is what `TokensValidAfterCache` compares against `tokens_valid_after`, so it has
    /// to survive the round trip — and land on the issuance instant, not on `exp` or the epoch.
    #[test]
    fn issuing_then_verifying_round_trips_the_issuance_instant() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let before = Utc::now();
        let token = issuer.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();
        let after = Utc::now();

        let claims = issuer.verify(&token).unwrap();

        assert!(claims.issued_at >= before - Duration::seconds(1), "issued_at {} predates issuance", claims.issued_at);
        assert!(claims.issued_at <= after, "issued_at {} postdates issuance", claims.issued_at);
    }

    /// A token minted before this change has no `iat` at all, and `deny_unknown_fields` aside, a
    /// missing required claim must fail closed rather than decode as the epoch (which would read as
    /// "issued in 1970" and be rejected by every `tokens_valid_after` check anyway — but loudly).
    #[test]
    fn a_token_without_an_iat_claim_does_not_verify() {
        #[derive(Serialize)]
        struct LegacyClaims {
            sub: Uuid,
            exp: i64,
            scope: Option<ScopeClaim>,
            typ: String,
            org: Uuid,
            super_admin: bool,
        }
        let secret = "test-secret".to_string();
        let legacy = LegacyClaims {
            sub: Uuid::new_v4(),
            exp: (Utc::now() + Duration::minutes(2)).timestamp(),
            scope: None,
            typ: DOCKER_ACCESS_TOKEN_TYPE.to_string(),
            org: Uuid::new_v4(),
            super_admin: false,
        };
        let token = encode(&Header::default(), &legacy, &EncodingKey::from_secret(&derive_signing_key(&secret, DOCKER_ACCESS_TOKEN_TYPE))).unwrap();

        assert!(JwtDockerTokenIssuer::new(secret).verify(&token).is_err());
    }

    #[test]
    fn a_token_issued_for_an_api_token_carries_its_id() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let api_token_id = Uuid::new_v4();

        let token = issuer.issue_for_api_token(api_token_id, Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();

        assert_eq!(issuer.verify(&token).unwrap().api_token_id, Some(api_token_id));
    }

    #[test]
    fn a_token_issued_without_an_api_token_carries_no_id() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());

        let token = issuer.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();

        assert_eq!(issuer.verify(&token).unwrap().api_token_id, None);
    }

    #[test]
    fn issuing_then_verifying_with_a_scope_round_trips_the_granted_actions() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let user_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let scope = DockerGrantedScope {
            resource_type: "repository".to_string(),
            name: "myrepo/myimage".to_string(),
            actions: vec!["pull".to_string()],
            granted_repository_id: Some(repository_id),
        };
        let token = issuer.issue(user_id, Uuid::new_v4(), false, Some(scope.clone())).unwrap();

        let claims = issuer.verify(&token).unwrap();

        let granted = claims.granted_scope.unwrap();
        assert_eq!(granted.actions, scope.actions);
        assert_eq!(granted.granted_repository_id, Some(repository_id));
    }

    #[test]
    fn issuing_then_verifying_round_trips_the_organization_and_super_admin_flag() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let organization_id = Uuid::new_v4();
        let token = issuer.issue(Uuid::new_v4(), organization_id, true, None).unwrap();

        let claims = issuer.verify(&token).unwrap();

        assert_eq!(claims.organization_id, organization_id);
        assert!(claims.is_super_admin);
    }

    #[test]
    fn verifying_a_token_signed_with_a_different_secret_fails() {
        let issuer_a = JwtDockerTokenIssuer::new("secret-a".to_string());
        let issuer_b = JwtDockerTokenIssuer::new("secret-b".to_string());
        let token = issuer_a.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();
        assert!(issuer_b.verify(&token).is_err());
    }

    #[test]
    fn a_session_token_is_rejected_by_the_docker_token_issuer() {
        let secret = "shared-secret".to_string();
        let session_issuer = crate::jwt_token_issuer::JwtTokenIssuer::new(secret.clone());
        let docker_issuer = JwtDockerTokenIssuer::new(secret);
        let session_token = artiferris_domain::user::TokenIssuerPort::issue(&session_issuer, Uuid::new_v4(), Duration::hours(12)).unwrap();

        assert!(docker_issuer.verify(&session_token).is_err(), "a session token must NOT verify as a docker access token");
    }

    #[test]
    fn a_token_lives_for_two_minutes() {
        let issuer = JwtDockerTokenIssuer::new("test-secret".to_string());
        let token = issuer.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();
        let claims = decode::<Claims>(&token, &DecodingKey::from_secret(&derive_signing_key("test-secret", DOCKER_ACCESS_TOKEN_TYPE)), &Validation::default()).unwrap().claims;

        assert_eq!(claims.exp - claims.iat, 120);
    }

    #[test]
    fn a_token_is_rejected_the_moment_it_expires_with_no_grace_period() {
        let expired = JwtDockerTokenIssuer::with_ttl("test-secret".to_string(), Duration::seconds(-10));
        let token = expired.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();

        assert!(JwtDockerTokenIssuer::new("test-secret".to_string()).verify(&token).is_err());
    }

    #[test]
    fn a_token_signed_with_the_raw_secret_instead_of_the_derived_key_is_rejected() {
        #[derive(Serialize)]
        struct RawClaims {
            sub: Uuid,
            iat: i64,
            exp: i64,
            scope: Option<ScopeClaim>,
            typ: String,
            org: Uuid,
            super_admin: bool,
        }
        let claims = RawClaims {
            sub: Uuid::new_v4(),
            iat: Utc::now().timestamp(),
            exp: (Utc::now() + Duration::minutes(2)).timestamp(),
            scope: None,
            typ: DOCKER_ACCESS_TOKEN_TYPE.to_string(),
            org: Uuid::new_v4(),
            super_admin: true,
        };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(b"test-secret")).unwrap();

        assert!(JwtDockerTokenIssuer::new("test-secret".to_string()).verify(&token).is_err());
    }
}
