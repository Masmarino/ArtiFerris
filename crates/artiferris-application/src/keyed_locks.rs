//! One async lock per key, so concurrent requests for the same thing take turns. Per process.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::OwnedMutexGuard;

pub struct KeyedLocks<K> {
    locks: Mutex<HashMap<K, Weak<tokio::sync::Mutex<()>>>>,
}

impl<K: Hash + Eq + Clone> KeyedLocks<K> {
    pub fn new() -> Self {
        Self { locks: Mutex::new(HashMap::new()) }
    }

    /// Waits for the lock on `key`. Entries nobody holds any more are dropped as new ones come in.
    pub async fn lock(&self, key: K) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            locks.retain(|_, lock| lock.strong_count() > 0);
            match locks.get(&key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(key, Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.locks.lock().unwrap().len()
    }
}

impl<K: Hash + Eq + Clone> Default for KeyedLocks<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_same_key_takes_turns_and_different_keys_do_not() {
        let locks = KeyedLocks::new();
        let first = locks.lock("a").await;

        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), locks.lock("a")).await.is_err(), "a second holder of the same key must wait");
        drop(tokio::time::timeout(std::time::Duration::from_millis(50), locks.lock("b")).await.expect("another key is free"));

        drop(first);
        tokio::time::timeout(std::time::Duration::from_millis(50), locks.lock("a")).await.expect("free once released");
    }

    #[tokio::test]
    async fn released_keys_are_forgotten() {
        let locks = KeyedLocks::new();
        for key in 0..100 {
            drop(locks.lock(key).await);
        }
        drop(locks.lock(100).await);

        assert!(locks.len() <= 2, "{} entries kept", locks.len());
    }
}
