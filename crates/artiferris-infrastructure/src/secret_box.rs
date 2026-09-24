//! Encrypts a secret at rest with AES-256-GCM, keyed from `SECRETS_ENCRYPTION_KEY` — a key
//! dedicated to this purpose, separate from `JWT_SECRET`.
//!
//! New values carry a version and a key id, and the column they live in is bound as associated
//! data, so a ciphertext copied into another column fails to open. Values written before that
//! format existed are still readable, and `reseal_*` upgrades them.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use artiferris_domain::error::DomainError;
use rand::Rng;
use sha2::Sha256;

pub const TOTP_SEED: &str = "totp_credentials.encrypted_secret";
pub const SMTP_PASSWORD: &str = "smtp_settings.encrypted_password";
pub const LDAP_BIND_PASSWORD: &str = "organization_identity_providers.config.bind_password";
pub const OIDC_CLIENT_SECRET: &str = "organization_identity_providers.config.client_secret";
pub const PROXY_REMOTE_PASSWORD: &str = "package_repositories.remote_password";

const NONCE_LEN: usize = 12;
const KEY_ID_LEN: usize = 4;
/// Leads a versioned ciphertext stored in a bytea column.
const SEALED_MAGIC: &[u8; 4] = b"AFS\x01";
/// Leads a versioned value stored in a text column; the legacy hex form can never contain a `.`.
const PACKED_PREFIX: &str = "af1.";

/// HKDF-SHA256 with a fixed info string. HKDF does not slow down guessing: the key has to be
/// high-entropy on its own (`openssl rand -base64 32`), a passphrase is not enough.
fn derive_key(secrets_encryption_key: &str) -> [u8; 32] {
    expand(secrets_encryption_key, b"artiferris-secret-box-v1")
}

/// Short fingerprint of the key stored next to each ciphertext, so a mismatch is reported as one and rotation can find what to re-encrypt.
fn key_id(secrets_encryption_key: &str) -> [u8; KEY_ID_LEN] {
    let full = expand(secrets_encryption_key, b"artiferris-secret-box-key-id");
    full[..KEY_ID_LEN].try_into().expect("slice has the key id length")
}

fn expand(secrets_encryption_key: &str, info: &[u8]) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(None, secrets_encryption_key.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

fn cipher(secrets_encryption_key: &str) -> Aes256Gcm {
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(derive_key(secrets_encryption_key)))
}

fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce);
    nonce
}

fn seal_raw(plaintext: &str, key: &str, context: &str) -> ([u8; NONCE_LEN], Vec<u8>) {
    let nonce_bytes = random_nonce();
    let ciphertext = cipher(key)
        .encrypt(&Nonce::from(nonce_bytes), Payload { msg: plaintext.as_bytes(), aad: context.as_bytes() })
        .expect("AES-GCM encryption of a bounded in-memory plaintext cannot fail");
    (nonce_bytes, ciphertext)
}

fn open_raw(key: &str, nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> Result<String, DomainError> {
    let nonce = Nonce::try_from(nonce).map_err(|_| DomainError::SecretUnreadable("stored nonce had an unexpected length".to_string()))?;
    let plaintext = cipher(key)
        .decrypt(&nonce, Payload { msg: ciphertext, aad })
        .map_err(|_| DomainError::SecretUnreadable("failed to decrypt with the current key".to_string()))?;
    String::from_utf8(plaintext).map_err(|_| DomainError::SecretUnreadable("decrypted secret was not valid UTF-8".to_string()))
}

fn hex_id(id: [u8; KEY_ID_LEN]) -> String {
    hex::encode(id)
}

/// The key ids are for whoever reads the log; the API never echoes this text (see `application_error_response`).
fn no_key_for(id: [u8; KEY_ID_LEN], keys: &[&str]) -> DomainError {
    let current = keys.first().map(|k| hex_id(key_id(k))).unwrap_or_default();
    DomainError::SecretUnreadable(format!("sealed with another SECRETS_ENCRYPTION_KEY (key id {}, this server's is {current})", hex_id(id)))
}

static PREVIOUS_KEY: OnceLock<String> = OnceLock::new();

/// Makes every later read fall back to `SECRETS_ENCRYPTION_KEY_PREVIOUS` for values still sealed before a rotation, not just the startup re-encryption.
/// Set once at startup; a second call is ignored.
pub fn set_previous_key(previous: &str) {
    let _ = PREVIOUS_KEY.set(previous.to_string());
}

fn runtime_keys(current: &str) -> Vec<&str> {
    std::iter::once(current).chain(PREVIOUS_KEY.get().map(String::as_str)).collect()
}

/// True the first time each kind of legacy value is seen, so it is logged once and not once per row.
fn first_legacy_read(context: &'static str, what: &'static str) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<(&'static str, &'static str)>>> = OnceLock::new();
    SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).insert((context, what))
}

fn warn_once(context: &'static str, what: &'static str) {
    if first_legacy_read(context, what) {
        tracing::warn!("a stored {context} is in a legacy {what} format; it is upgraded at a startup with SECRETS_REENCRYPT_LEGACY=true");
    }
}

/// Returns `(ciphertext, nonce)` for a pair of bytea columns. The ciphertext column carries the version and key id.
pub fn seal(plaintext: &str, key: &str, context: &'static str) -> (Vec<u8>, Vec<u8>) {
    let (nonce, ciphertext) = seal_raw(plaintext, key, context);
    let mut stored = Vec::with_capacity(SEALED_MAGIC.len() + KEY_ID_LEN + ciphertext.len());
    stored.extend_from_slice(SEALED_MAGIC);
    stored.extend_from_slice(&key_id(key));
    stored.extend_from_slice(&ciphertext);
    (stored, nonce.to_vec())
}

pub fn open(ciphertext: &[u8], nonce: &[u8], key: &str, context: &'static str) -> Result<String, DomainError> {
    open_with(ciphertext, nonce, &runtime_keys(key), context)
}

/// `keys[0]` is the current key; the rest are tried for values sealed before a rotation.
fn open_with(stored: &[u8], nonce: &[u8], keys: &[&str], context: &'static str) -> Result<String, DomainError> {
    let versioned = stored.strip_prefix(SEALED_MAGIC.as_slice()).filter(|rest| rest.len() > KEY_ID_LEN);
    let mut first_error = None;
    if let Some(rest) = versioned {
        let (id, ciphertext) = rest.split_at(KEY_ID_LEN);
        let id: [u8; KEY_ID_LEN] = id.try_into().expect("split at the key id length");
        match keys.iter().find(|k| key_id(k) == id) {
            Some(key) => match open_raw(key, nonce, ciphertext, context.as_bytes()) {
                Ok(plaintext) => return Ok(plaintext),
                Err(e) => first_error = Some(e),
            },
            None => first_error = Some(no_key_for(id, keys)),
        }
    }
    // Not versioned (or, with odds of 1 in 2^32, a legacy ciphertext that happens to start like one).
    for key in keys {
        match open_raw(key, nonce, stored, b"") {
            Ok(plaintext) => {
                warn_once(context, "ciphertext");
                return Ok(plaintext);
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.expect("at least one key was tried"))
}

/// One text value for columns and JSON fields that hold a single string: `af1.<key id>.<hex(nonce ‖ ciphertext)>`.
pub fn seal_packed(plaintext: &str, key: &str, context: &'static str) -> String {
    let (nonce, ciphertext) = seal_raw(plaintext, key, context);
    format!("{PACKED_PREFIX}{}.{}", hex_id(key_id(key)), hex::encode([nonce.as_slice(), ciphertext.as_slice()].concat()))
}

pub fn open_packed(packed: &str, key: &str, context: &'static str) -> Result<String, DomainError> {
    open_packed_with(packed, &runtime_keys(key), context)
}

fn open_packed_with(packed: &str, keys: &[&str], context: &'static str) -> Result<String, DomainError> {
    if let Some(rest) = packed.strip_prefix(PACKED_PREFIX) {
        let (id, body) = parse_packed_body(rest)?;
        let (nonce, ciphertext) = body.split_at(NONCE_LEN);
        let key = keys.iter().find(|k| key_id(k) == id).ok_or_else(|| no_key_for(id, keys))?;
        return open_raw(key, nonce, ciphertext, context.as_bytes());
    }
    // Legacy: bare hex of nonce ‖ ciphertext. Anything that is not structurally that was stored as plaintext before encryption existed.
    match hex::decode(packed) {
        Ok(bytes) if bytes.len() >= NONCE_LEN => {
            let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
            let mut first_error = None;
            for key in keys {
                match open_raw(key, nonce, ciphertext, b"") {
                    Ok(plaintext) => {
                        warn_once(context, "ciphertext");
                        return Ok(plaintext);
                    }
                    Err(e) => {
                        first_error.get_or_insert(e);
                    }
                }
            }
            Err(first_error.expect("at least one key was tried"))
        }
        _ => {
            warn_once(context, "plaintext");
            Ok(packed.to_string())
        }
    }
}

fn parse_packed_body(rest: &str) -> Result<([u8; KEY_ID_LEN], Vec<u8>), DomainError> {
    let malformed = || DomainError::SecretUnreadable("malformed".to_string());
    let (id, body) = rest.split_once('.').ok_or_else(malformed)?;
    let id: [u8; KEY_ID_LEN] = hex::decode(id).map_err(|_| malformed())?.try_into().map_err(|_| malformed())?;
    let body = hex::decode(body).map_err(|_| malformed())?;
    if body.len() <= NONCE_LEN {
        return Err(malformed());
    }
    Ok((id, body))
}

/// Whether `packed` is already in the current format under `key`.
fn packed_is_current(packed: &str, key: &str) -> bool {
    packed.strip_prefix(PACKED_PREFIX).and_then(|rest| parse_packed_body(rest).ok()).is_some_and(|(id, _)| id == key_id(key))
}

fn pair_is_current(stored: &[u8], key: &str) -> bool {
    stored.strip_prefix(SEALED_MAGIC.as_slice()).is_some_and(|rest| rest.len() > KEY_ID_LEN && rest[..KEY_ID_LEN] == key_id(key))
}

/// `None` when the value is already sealed under `key`; otherwise the same secret in the current format.
/// `previous` is the key in use before a rotation.
pub fn reseal_packed(packed: &str, key: &str, previous: Option<&str>, context: &'static str) -> Result<Option<String>, DomainError> {
    if packed_is_current(packed, key) {
        return Ok(None);
    }
    let keys: Vec<&str> = std::iter::once(key).chain(previous).collect();
    Ok(Some(seal_packed(&open_packed_with(packed, &keys, context)?, key, context)))
}

pub fn reseal(ciphertext: &[u8], nonce: &[u8], key: &str, previous: Option<&str>, context: &'static str) -> Result<Option<(Vec<u8>, Vec<u8>)>, DomainError> {
    if pair_is_current(ciphertext, key) {
        return Ok(None);
    }
    let keys: Vec<&str> = std::iter::once(key).chain(previous).collect();
    Ok(Some(seal(&open_with(ciphertext, nonce, &keys, context)?, key, context)))
}

#[cfg(test)]
pub(crate) fn legacy_encrypt(plaintext: &str, key: &str) -> (Vec<u8>, Vec<u8>) {
    let nonce = random_nonce();
    let ciphertext = cipher(key).encrypt(&Nonce::from(nonce), plaintext.as_bytes()).unwrap();
    (ciphertext, nonce.to_vec())
}

#[cfg(test)]
pub(crate) fn legacy_encrypt_packed(plaintext: &str, key: &str) -> String {
    let (ciphertext, nonce) = legacy_encrypt(plaintext, key);
    hex::encode([nonce, ciphertext].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_packed_prefix_matches_the_one_the_audit_trail_redacts() {
        assert_eq!(PACKED_PREFIX, artiferris_domain::audit::SEALED_SECRET_PREFIX);
        assert!(seal_packed("x", "key-a", CTX).starts_with(artiferris_domain::audit::SEALED_SECRET_PREFIX));
    }

    const CTX: &str = "test.column";

    #[test]
    fn sealing_then_opening_returns_the_original_plaintext() {
        let (ciphertext, nonce) = seal("hunter2", "secret-key", CTX);
        assert_eq!(open(&ciphertext, &nonce, "secret-key", CTX).unwrap(), "hunter2");
    }

    #[test]
    fn opening_with_the_wrong_key_fails() {
        let (ciphertext, nonce) = seal("hunter2", "key-a", CTX);
        assert!(open(&ciphertext, &nonce, "key-b", CTX).is_err());
    }

    #[test]
    fn a_value_sealed_with_another_key_is_reported_as_such() {
        let error = open_packed(&seal_packed("hunter2", "key-a", CTX), "key-b", CTX).unwrap_err().to_string();
        assert!(error.contains("another SECRETS_ENCRYPTION_KEY"), "got: {error}");
    }

    #[test]
    fn reads_fall_back_to_the_previous_key_once_it_is_configured() {
        let previous = "previous-key-only-this-test-configures";
        let (ciphertext, nonce) = seal("hunter2", previous, CTX);
        let packed = seal_packed("hunter2", previous, CTX);
        assert!(open(&ciphertext, &nonce, "rotated-current-key", CTX).is_err());
        assert!(open_packed(&packed, "rotated-current-key", CTX).is_err());

        set_previous_key(previous);

        assert_eq!(open(&ciphertext, &nonce, "rotated-current-key", CTX).unwrap(), "hunter2");
        assert_eq!(open_packed(&packed, "rotated-current-key", CTX).unwrap(), "hunter2");
        let (current, current_nonce) = seal("fresh", "rotated-current-key", CTX);
        assert_eq!(open(&current, &current_nonce, "rotated-current-key", CTX).unwrap(), "fresh", "values under the current key are untouched");
        let (stranger, stranger_nonce) = seal("hunter2", "some-third-key", CTX);
        assert!(open(&stranger, &stranger_nonce, "rotated-current-key", CTX).is_err(), "only the configured previous key is a fallback");
    }

    #[test]
    fn two_seals_of_the_same_plaintext_use_different_nonces() {
        let (ciphertext_a, nonce_a) = seal("hunter2", "secret-key", CTX);
        let (ciphertext_b, nonce_b) = seal("hunter2", "secret-key", CTX);
        assert_ne!(nonce_a, nonce_b, "a reused nonce would break AES-GCM's security guarantees");
        assert_ne!(ciphertext_a, ciphertext_b);
    }

    #[test]
    fn packed_sealing_round_trips_and_carries_a_version_prefix() {
        let packed = seal_packed("hunter2", "secret-key", CTX);
        assert!(packed.starts_with("af1."));
        assert_eq!(open_packed(&packed, "secret-key", CTX).unwrap(), "hunter2");
    }

    #[test]
    fn a_packed_value_never_contains_the_plaintext() {
        assert!(!seal_packed("hunter2", "secret-key", CTX).contains("hunter2"));
    }

    #[test]
    fn a_ciphertext_moved_to_another_column_does_not_open() {
        let (ciphertext, nonce) = seal("hunter2", "secret-key", TOTP_SEED);
        assert!(open(&ciphertext, &nonce, "secret-key", SMTP_PASSWORD).is_err());
        let packed = seal_packed("hunter2", "secret-key", LDAP_BIND_PASSWORD);
        assert!(open_packed(&packed, "secret-key", OIDC_CLIENT_SECRET).is_err());
    }

    #[test]
    fn a_tampered_packed_value_fails_to_open() {
        let mut packed = seal_packed("hunter2", "secret-key", CTX);
        let last = packed.pop().unwrap();
        packed.push(if last == '0' { '1' } else { '0' });
        assert!(open_packed(&packed, "secret-key", CTX).is_err());
        assert!(open_packed("af1.zz", "secret-key", CTX).is_err());
        assert!(open_packed("af1.", "secret-key", CTX).is_err());
    }

    #[test]
    fn legacy_ciphertexts_are_still_readable() {
        let (ciphertext, nonce) = legacy_encrypt("hunter2", "secret-key");
        assert_eq!(open(&ciphertext, &nonce, "secret-key", CTX).unwrap(), "hunter2");
        assert_eq!(open_packed(&legacy_encrypt_packed("hunter2", "secret-key"), "secret-key", CTX).unwrap(), "hunter2");
    }

    #[test]
    fn a_legacy_plaintext_value_is_read_as_is() {
        assert_eq!(open_packed("not-encrypted-at-all", "secret-key", CTX).unwrap(), "not-encrypted-at-all");
    }

    #[test]
    fn a_legacy_value_that_parses_as_ciphertext_but_does_not_open_is_a_hard_error() {
        let packed = legacy_encrypt_packed("hunter2", "key-a");
        assert!(open_packed(&packed, "key-b", CTX).is_err(), "a key mismatch must never fall back to garbage plaintext");
    }

    #[test]
    fn the_legacy_warning_fires_once_per_kind_of_value() {
        assert!(first_legacy_read("test.warn_once_column", "plaintext"));
        assert!(!first_legacy_read("test.warn_once_column", "plaintext"));
        assert!(first_legacy_read("test.warn_once_column", "ciphertext"));
        assert!(first_legacy_read("test.other_column", "plaintext"));
    }

    #[test]
    fn the_key_id_identifies_the_key_without_revealing_it() {
        assert_eq!(key_id("key-a"), key_id("key-a"));
        assert_ne!(key_id("key-a"), key_id("key-b"));
        assert_ne!(&derive_key("key-a")[..KEY_ID_LEN], &key_id("key-a")[..], "the id must not be a prefix of the encryption key");
    }

    #[test]
    fn resealing_upgrades_legacy_values_and_leaves_current_ones_alone() {
        let legacy = legacy_encrypt_packed("hunter2", "secret-key");
        let upgraded = reseal_packed(&legacy, "secret-key", None, CTX).unwrap().expect("legacy needs an upgrade");
        assert!(upgraded.starts_with("af1."));
        assert_eq!(open_packed(&upgraded, "secret-key", CTX).unwrap(), "hunter2");
        assert!(reseal_packed(&upgraded, "secret-key", None, CTX).unwrap().is_none());

        let plaintext = reseal_packed("plain-password", "secret-key", None, CTX).unwrap().expect("plaintext needs an upgrade");
        assert_eq!(open_packed(&plaintext, "secret-key", CTX).unwrap(), "plain-password");

        let (legacy_ct, legacy_nonce) = legacy_encrypt("hunter2", "secret-key");
        let (ciphertext, nonce) = reseal(&legacy_ct, &legacy_nonce, "secret-key", None, CTX).unwrap().expect("legacy needs an upgrade");
        assert_eq!(open(&ciphertext, &nonce, "secret-key", CTX).unwrap(), "hunter2");
        assert!(reseal(&ciphertext, &nonce, "secret-key", None, CTX).unwrap().is_none());
    }

    #[test]
    fn resealing_moves_values_from_the_previous_key_to_the_new_one() {
        let old_sealed = seal_packed("hunter2", "old-key", CTX);
        assert!(reseal_packed(&old_sealed, "new-key", None, CTX).is_err(), "without the previous key the value cannot be moved");
        let moved = reseal_packed(&old_sealed, "new-key", Some("old-key"), CTX).unwrap().unwrap();
        assert_eq!(open_packed(&moved, "new-key", CTX).unwrap(), "hunter2");

        let (ciphertext, nonce) = seal("hunter2", "old-key", CTX);
        let (moved_ct, moved_nonce) = reseal(&ciphertext, &nonce, "new-key", Some("old-key"), CTX).unwrap().unwrap();
        assert_eq!(open(&moved_ct, &moved_nonce, "new-key", CTX).unwrap(), "hunter2");

        let legacy_old = legacy_encrypt_packed("hunter2", "old-key");
        let moved = reseal_packed(&legacy_old, "new-key", Some("old-key"), CTX).unwrap().unwrap();
        assert_eq!(open_packed(&moved, "new-key", CTX).unwrap(), "hunter2");
    }

    #[test]
    fn the_kdf_is_not_raw_single_round_sha256() {
        let old_style = { use sha2::{Digest, Sha256}; let d: [u8; 32] = Sha256::digest(b"some-key").into(); d };
        assert_ne!(old_style, derive_key("some-key"), "the key derivation must not be raw SHA-256");
    }
}
