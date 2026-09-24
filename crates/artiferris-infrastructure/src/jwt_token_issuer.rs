use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use artiferris_domain::error::DomainError;
use artiferris_domain::user::{TokenIssuerPort, VerifiedToken};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error_ext::InfraErr;
use crate::token_keys::derive_signing_key;

const SESSION_TOKEN_TYPE: &str = "session";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: Uuid,
    exp: i64,
    iat: i64,
    typ: String,
}

pub struct JwtTokenIssuer {
    key: [u8; 32],
}

impl JwtTokenIssuer {
    pub fn new(secret: String) -> Self {
        Self { key: derive_signing_key(&secret, SESSION_TOKEN_TYPE) }
    }
}

#[async_trait]
impl TokenIssuerPort for JwtTokenIssuer {
    fn issue(&self, user_id: Uuid, ttl: Duration) -> Result<String, DomainError> {
        let now = Utc::now();
        let claims = Claims { sub: user_id, exp: (now + ttl).timestamp(), iat: now.timestamp(), typ: SESSION_TOKEN_TYPE.to_string() };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(&self.key)).infra_err()
    }

    fn verify(&self, token: &str) -> Result<VerifiedToken, DomainError> {
        let data = decode::<Claims>(token, &DecodingKey::from_secret(&self.key), &Validation::default()).infra_err()?;
        if data.claims.typ != SESSION_TOKEN_TYPE {
            return Err(DomainError::Infrastructure("not a session token".to_string()));
        }
        let issued_at =
            DateTime::from_timestamp(data.claims.iat, 0).ok_or_else(|| DomainError::Infrastructure("invalid token: bad iat".to_string()))?;
        Ok(VerifiedToken { user_id: data.claims.sub, issued_at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuing_then_verifying_returns_the_same_user_id() {
        let issuer = JwtTokenIssuer::new("test-secret".to_string());
        let user_id = Uuid::new_v4();
        let token = issuer.issue(user_id, Duration::hours(12)).unwrap();
        assert_eq!(issuer.verify(&token).unwrap().user_id, user_id);
    }

    #[test]
    fn a_verified_token_reports_when_it_was_issued() {
        let issuer = JwtTokenIssuer::new("test-secret".to_string());
        let before = Utc::now();
        let token = issuer.issue(Uuid::new_v4(), Duration::hours(12)).unwrap();
        let verified = issuer.verify(&token).unwrap();
        assert!(verified.issued_at >= before - Duration::seconds(1));
        assert!(verified.issued_at <= Utc::now() + Duration::seconds(1));
    }

    #[test]
    fn a_negative_ttl_produces_an_already_expired_token() {
        let issuer = JwtTokenIssuer::new("test-secret".to_string());
        let token = issuer.issue(Uuid::new_v4(), Duration::seconds(-300)).unwrap();
        assert!(issuer.verify(&token).is_err(), "a token issued with a negative TTL must already be expired");
    }

    #[test]
    fn verifying_a_token_signed_with_a_different_secret_fails() {
        let issuer_a = JwtTokenIssuer::new("secret-a".to_string());
        let issuer_b = JwtTokenIssuer::new("secret-b".to_string());
        let token = issuer_a.issue(Uuid::new_v4(), Duration::hours(12)).unwrap();
        assert!(issuer_b.verify(&token).is_err());
    }

    #[test]
    fn a_docker_access_token_is_rejected_by_the_session_token_issuer() {
        let secret = "shared-secret".to_string();
        let session_issuer = JwtTokenIssuer::new(secret.clone());
        let docker_issuer = crate::jwt_docker_token_issuer::JwtDockerTokenIssuer::new(secret);
        let docker_token = artiferris_domain::docker_registry::DockerTokenIssuerPort::issue(&docker_issuer, Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();

        assert!(session_issuer.verify(&docker_token).is_err(), "a docker access token must NOT verify as a session token");
    }

    #[test]
    fn a_token_signed_with_the_raw_secret_instead_of_the_derived_key_is_rejected() {
        let claims = Claims { sub: Uuid::new_v4(), exp: (Utc::now() + Duration::hours(1)).timestamp(), iat: Utc::now().timestamp(), typ: SESSION_TOKEN_TYPE.to_string() };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(b"test-secret")).unwrap();

        assert!(JwtTokenIssuer::new("test-secret".to_string()).verify(&token).is_err());
    }

    #[test]
    fn a_token_with_the_right_key_but_the_wrong_type_is_rejected() {
        let issuer = JwtTokenIssuer::new("test-secret".to_string());
        let claims = Claims { sub: Uuid::new_v4(), exp: (Utc::now() + Duration::hours(1)).timestamp(), iat: Utc::now().timestamp(), typ: "mfa-pending".to_string() };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(&issuer.key)).unwrap();

        assert!(issuer.verify(&token).is_err());
    }
}
