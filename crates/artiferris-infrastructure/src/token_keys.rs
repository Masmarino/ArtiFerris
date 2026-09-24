use sha2::Sha256;

/// One signing key per token type, all derived from `JWT_SECRET`. A token minted for one purpose can't verify as another, whatever claims it carries.
pub(crate) fn derive_signing_key(jwt_secret: &str, purpose: &str) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(None, jwt_secret.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(format!("artiferris-jwt-{purpose}-v1").as_bytes(), &mut okm).expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_secret_and_purpose_always_derive_the_same_key() {
        assert_eq!(derive_signing_key("secret", "session"), derive_signing_key("secret", "session"));
    }

    #[test]
    fn different_purposes_derive_different_keys() {
        assert_ne!(derive_signing_key("secret", "session"), derive_signing_key("secret", "docker-access"));
    }

    #[test]
    fn different_secrets_derive_different_keys() {
        assert_ne!(derive_signing_key("secret-a", "session"), derive_signing_key("secret-b", "session"));
    }
}
