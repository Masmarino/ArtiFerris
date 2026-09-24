//! Keeps one client from inflating the popularity counters by pulling the same thing over and over.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::Utc;
use uuid::Uuid;

/// Bounds memory when an unbounded number of distinct clients show up within one hour.
const MAX_TRACKED_KEYS: usize = 200_000;

/// What one client bucket can occupy of the table per hour, so a single address range cannot fill it for everyone else.
const MAX_KEYS_PER_CLIENT: usize = 1_000;

const HOUR_SECONDS: i64 = 3600;

type Key = (String, Uuid, String);

/// A client bucket counts at most once per repository and name per hour. Lives in memory only and is never persisted:
/// no address ever reaches the database.
#[derive(Default)]
pub struct DownloadDedupe {
    seen: Mutex<HourlyKeys>,
}

/// What was seen during one hour; a new hour starts an empty set.
#[derive(Default)]
struct HourlyKeys {
    hour: i64,
    keys: HashSet<Key>,
    per_client: HashMap<String, usize>,
}

impl DownloadDedupe {
    pub fn new() -> Self {
        Self::default()
    }

    /// True the first time `client` is seen for this repository and name in the current hour. A client past its share of the
    /// table, or any client once the table is full, goes uncounted until the next hour rather than let a flood inflate the counters.
    pub fn first_in_hour(&self, client: &str, repository_id: Uuid, name: &str) -> bool {
        self.first_in_hour_at(Utc::now().timestamp() / HOUR_SECONDS, client, repository_id, name)
    }

    fn first_in_hour_at(&self, hour: i64, client: &str, repository_id: Uuid, name: &str) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if seen.hour != hour {
            seen.keys.clear();
            seen.per_client.clear();
            seen.hour = hour;
        }
        let key = (client.to_string(), repository_id, name.to_string());
        if seen.keys.contains(&key) || seen.keys.len() >= MAX_TRACKED_KEYS || seen.per_client.get(client).is_some_and(|used| *used >= MAX_KEYS_PER_CLIENT) {
            return false;
        }
        *seen.per_client.entry(client.to_string()).or_default() += 1;
        seen.keys.insert(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_counts_once_per_repository_and_name_per_hour() {
        let dedupe = DownloadDedupe::new();
        let repository = Uuid::new_v4();

        assert!(dedupe.first_in_hour_at(10, "203.0.113.7", repository, "left-pad"));
        assert!(!dedupe.first_in_hour_at(10, "203.0.113.7", repository, "left-pad"));
        assert!(dedupe.first_in_hour_at(10, "203.0.113.8", repository, "left-pad"), "another client counts");
        assert!(dedupe.first_in_hour_at(10, "203.0.113.7", repository, "right-pad"), "another name counts");
        assert!(dedupe.first_in_hour_at(10, "203.0.113.7", Uuid::new_v4(), "left-pad"), "another repository counts");
        assert!(dedupe.first_in_hour_at(11, "203.0.113.7", repository, "left-pad"), "the next hour counts again");
    }

    #[test]
    fn one_client_cannot_use_more_than_its_share_of_the_table_and_others_still_count() {
        let dedupe = DownloadDedupe::new();
        let repository = Uuid::new_v4();
        for i in 0..MAX_KEYS_PER_CLIENT {
            assert!(dedupe.first_in_hour_at(5, "203.0.113.7", repository, &format!("pkg-{i}")));
        }

        assert!(!dedupe.first_in_hour_at(5, "203.0.113.7", repository, "one-more"), "past its share");
        assert!(dedupe.first_in_hour_at(5, "203.0.113.8", repository, "one-more"), "another client is not affected");
        assert!(dedupe.first_in_hour_at(6, "203.0.113.7", repository, "one-more"), "the next hour starts over");
        assert_eq!(dedupe.seen.lock().unwrap().per_client.len(), 1);
    }

    #[test]
    fn the_table_stays_bounded_and_refuses_new_clients_once_full() {
        let dedupe = DownloadDedupe::new();
        let repository = Uuid::new_v4();
        for i in 0..MAX_TRACKED_KEYS {
            assert!(dedupe.first_in_hour_at(5, &format!("client-{i}"), repository, "pkg"));
        }

        assert!(!dedupe.first_in_hour_at(5, "one-more", repository, "pkg"));
        assert!(dedupe.first_in_hour_at(6, "one-more", repository, "pkg"), "a new hour frees the old entries");
        assert!(dedupe.seen.lock().unwrap().keys.len() <= MAX_TRACKED_KEYS);
    }

    #[test]
    fn a_full_table_answers_new_clients_without_scanning_it() {
        let dedupe = DownloadDedupe::new();
        let repository = Uuid::new_v4();
        for i in 0..MAX_TRACKED_KEYS {
            dedupe.first_in_hour_at(5, &format!("client-{i}"), repository, "pkg");
        }

        let started = std::time::Instant::now();
        for i in 0..50_000 {
            assert!(!dedupe.first_in_hour_at(5, &format!("flood-{i}"), repository, "pkg"));
        }

        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        assert!(!dedupe.first_in_hour_at(5, "client-0", repository, "pkg"), "clients already counted this hour stay counted");
    }

    #[test]
    fn last_hours_entries_are_forgotten_when_the_hour_turns() {
        let dedupe = DownloadDedupe::new();
        let repository = Uuid::new_v4();
        assert!(dedupe.first_in_hour_at(7, "a", repository, "pkg"));

        assert!(dedupe.first_in_hour_at(8, "a", repository, "pkg"));
        assert_eq!(dedupe.seen.lock().unwrap().keys.len(), 1);
    }
}
