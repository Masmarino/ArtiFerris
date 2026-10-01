use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use artiferris_domain::error::DomainError;
use artiferris_domain::user::{TokenIssuerPort, VerifiedToken};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error_ext::InfraErr;
use crate::token_keys::derive_signing_key;

const MFA_PENDING_TOKEN_TYPE: &str = "mfa-pending";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: Uuid,
    exp: i64,
    iat: i64,
    typ: String,
    /// Makes two tokens minted in the same second differ, so the API's single-use tracking can tell them apart.
    jti: Uuid,
}

pub struct JwtMfaPendingTokenIssuer {
    key: [u8; 32],
    ttl: Duration,
}

impl JwtMfaPendingTokenIssuer {
    pub fn new(secret: String) -> Self {
        Self { key: derive_signing_key(&secret, MFA_PENDING_TOKEN_TYPE), ttl: Duration::minutes(5) }
    }
}

#[async_trait]
impl TokenIssuerPort for JwtMfaPendingTokenIssuer {
    // `ttl` is ignored: an mfa-pending token is a short-lived proof that the password was verified, with its own fixed
    // 5-minute lifetime.
    fn issue(&self, user_id: Uuid, _ttl: Duration) -> Result<String, DomainError> {
        let now = Utc::now();
        let claims = Claims { sub: user_id, exp: (now + self.ttl).timestamp(), iat: now.timestamp(), typ: MFA_PENDING_TOKEN_TYPE.to_string(), jti: Uuid::new_v4() };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(&self.key)).infra_err()
    }

    fn verify(&self, token: &str) -> Result<VerifiedToken, DomainError> {
        let data = decode::<Claims>(token, &DecodingKey::from_secret(&self.key), &Validation::default()).infra_err()?;
        if data.claims.typ != MFA_PENDING_TOKEN_TYPE {
            return Err(DomainError::Infrastructure("not an mfa-pending token".to_string()));
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
        let issuer = JwtMfaPendingTokenIssuer::new("test-secret".to_string());
        let user_id = Uuid::new_v4();
        let token = issuer.issue(user_id, Duration::minutes(5)).unwrap();
        assert_eq!(issuer.verify(&token).unwrap().user_id, user_id);
    }

    #[test]
    fn verifying_a_token_signed_with_a_different_secret_fails() {
        let issuer_a = JwtMfaPendingTokenIssuer::new("secret-a".to_string());
        let issuer_b = JwtMfaPendingTokenIssuer::new("secret-b".to_string());
        let token = issuer_a.issue(Uuid::new_v4(), Duration::minutes(5)).unwrap();
        assert!(issuer_b.verify(&token).is_err());
    }

    #[test]
    fn a_session_token_is_rejected_by_the_mfa_pending_token_issuer() {
        let secret = "shared-secret".to_string();
        let session_issuer = crate::jwt_token_issuer::JwtTokenIssuer::new(secret.clone());
        let mfa_issuer = JwtMfaPendingTokenIssuer::new(secret);
        let session_token = session_issuer.issue(Uuid::new_v4(), Duration::hours(12)).unwrap();

        assert!(mfa_issuer.verify(&session_token).is_err(), "a session token must NOT verify as an mfa-pending token");
    }

    #[test]
    fn an_mfa_pending_token_is_rejected_by_the_session_token_issuer() {
        let secret = "shared-secret".to_string();
        let session_issuer = crate::jwt_token_issuer::JwtTokenIssuer::new(secret.clone());
        let mfa_issuer = JwtMfaPendingTokenIssuer::new(secret);
        let mfa_token = mfa_issuer.issue(Uuid::new_v4(), Duration::minutes(5)).unwrap();

        assert!(session_issuer.verify(&mfa_token).is_err(), "an mfa-pending token must NOT verify as a session token");
    }

    #[test]
    fn two_tokens_minted_in_the_same_second_for_one_user_differ() {
        let issuer = JwtMfaPendingTokenIssuer::new("test-secret".to_string());
        let user_id = Uuid::new_v4();

        assert_ne!(issuer.issue(user_id, Duration::minutes(5)).unwrap(), issuer.issue(user_id, Duration::minutes(5)).unwrap());
    }

    #[test]
    fn a_token_signed_with_the_raw_secret_instead_of_the_derived_key_is_rejected() {
        let claims = Claims {
            sub: Uuid::new_v4(),
            exp: (Utc::now() + Duration::minutes(5)).timestamp(),
            iat: Utc::now().timestamp(),
            typ: MFA_PENDING_TOKEN_TYPE.to_string(),
            jti: Uuid::new_v4(),
        };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(b"test-secret")).unwrap();

        assert!(JwtMfaPendingTokenIssuer::new("test-secret".to_string()).verify(&token).is_err());
    }

    #[test]
    fn a_token_with_the_right_key_but_the_wrong_type_is_rejected() {
        let issuer = JwtMfaPendingTokenIssuer::new("test-secret".to_string());
        let claims = Claims { sub: Uuid::new_v4(), exp: (Utc::now() + Duration::minutes(5)).timestamp(), iat: Utc::now().timestamp(), typ: "session".to_string(), jti: Uuid::new_v4() };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(&issuer.key)).unwrap();

        assert!(issuer.verify(&token).is_err());
    }
}
