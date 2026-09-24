use std::sync::Arc;
use std::time::Duration;

use argon2::password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::user::PasswordHasherPort;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error_ext::InfraErr;

/// How long a hash or verify waits for a free slot before the request is turned away.
const MAX_WAIT: Duration = Duration::from_secs(2);

/// Each hash takes 19 MiB and a core for tens of milliseconds, so the number running at once is capped.
pub struct Argon2PasswordHasher {
    slots: Arc<Semaphore>,
    max_wait: Duration,
}

impl Argon2PasswordHasher {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
        Self::with_limits(cores.max(2), MAX_WAIT)
    }

    fn with_limits(concurrency: usize, max_wait: Duration) -> Self {
        Self { slots: Arc::new(Semaphore::new(concurrency)), max_wait }
    }

    async fn slot(&self) -> Result<OwnedSemaphorePermit, DomainError> {
        match tokio::time::timeout(self.max_wait, self.slots.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(DomainError::Busy("too many passwords are being checked at once".to_string())),
        }
    }
}

impl Default for Argon2PasswordHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PasswordHasherPort for Argon2PasswordHasher {
    async fn hash(&self, plain_password: &str) -> Result<String, DomainError> {
        let permit = self.slot().await?;
        let plain_password = plain_password.to_string();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Argon2::default().hash_password(plain_password.as_bytes()).map(|hash| hash.to_string()).infra_err()
        })
        .await
        .map_err(|e| DomainError::Infrastructure(format!("password hashing task panicked: {e}")))?
    }

    async fn verify(&self, plain_password: &str, hash: &str) -> Result<bool, DomainError> {
        let permit = self.slot().await?;
        let plain_password = plain_password.to_string();
        let hash = hash.to_string();
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let Ok(parsed_hash) = PasswordHash::new(&hash) else {
                return false;
            };
            Argon2::default().verify_password(plain_password.as_bytes(), &parsed_hash).is_ok()
        })
        .await
        .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashing_then_verifying_the_same_password_succeeds() {
        let hasher = Argon2PasswordHasher::new();
        let hash = hasher.hash("sup3r-s3cret!").await.unwrap();
        assert!(hasher.verify("sup3r-s3cret!", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn verifying_a_wrong_password_fails() {
        let hasher = Argon2PasswordHasher::new();
        let hash = hasher.hash("sup3r-s3cret!").await.unwrap();
        assert!(!hasher.verify("wrong-password", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn with_every_slot_taken_a_hash_or_verify_is_turned_away_after_the_wait_instead_of_queueing_forever() {
        let hasher = Argon2PasswordHasher::with_limits(1, Duration::from_millis(50));
        let hash = hasher.hash("sup3r-s3cret!").await.unwrap();
        let held = hasher.slots.clone().acquire_owned().await.unwrap();

        let started = std::time::Instant::now();
        assert!(matches!(hasher.hash("another").await, Err(DomainError::Busy(_))));
        assert!(matches!(hasher.verify("sup3r-s3cret!", &hash).await, Err(DomainError::Busy(_))));
        assert!(started.elapsed() < Duration::from_secs(1), "the wait is bounded");

        drop(held);
        assert!(hasher.verify("sup3r-s3cret!", &hash).await.unwrap(), "a freed slot serves the next caller");
    }

    #[tokio::test]
    async fn a_caller_waits_for_a_slot_that_frees_up_in_time() {
        let hasher = Arc::new(Argon2PasswordHasher::with_limits(1, Duration::from_secs(5)));
        let hash = hasher.hash("sup3r-s3cret!").await.unwrap();
        let held = hasher.slots.clone().acquire_owned().await.unwrap();
        let waiting = tokio::spawn({
            let hasher = hasher.clone();
            async move { hasher.verify("sup3r-s3cret!", &hash).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        drop(held);

        assert!(waiting.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn a_slot_is_given_back_when_the_hash_finishes_so_a_burst_never_wedges_the_hasher() {
        let hasher = Arc::new(Argon2PasswordHasher::with_limits(2, Duration::from_secs(30)));
        let hash = hasher.hash("sup3r-s3cret!").await.unwrap();

        let checks: Vec<_> = (0..12)
            .map(|_| {
                let (hasher, hash) = (hasher.clone(), hash.clone());
                tokio::spawn(async move { hasher.verify("sup3r-s3cret!", &hash).await })
            })
            .collect();

        for check in checks {
            assert!(check.await.unwrap().unwrap());
        }
        assert_eq!(hasher.slots.available_permits(), 2);
    }
}
