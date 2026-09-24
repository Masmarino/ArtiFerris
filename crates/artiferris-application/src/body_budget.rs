//! A shared byte budget for request bodies that have to be held in memory at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Budget accounting granularity: a semaphore counts permits in a `u32`, so bytes are rounded up to KiB.
const UNIT_BYTES: usize = 1024;

/// Bytes handed out by default across all in-flight bodies.
pub const DEFAULT_BODY_BUDGET_BYTES: usize = 1024 * 1024 * 1024;

/// Bodies one client, or one user, may have in flight at a time.
pub const MAX_BODIES_PER_CLIENT: usize = 4;

/// What an admitted body holds before any of it has arrived.
const INITIAL_RESERVATION_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct BodyBudget {
    semaphore: Arc<Semaphore>,
    in_flight: Arc<Mutex<HashMap<String, usize>>>,
}

/// Why a body wasn't admitted.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The budget can't cover it right now.
    Busy,
    /// The client or user already has `MAX_BODIES_PER_CLIENT` bodies in flight.
    TooManyFromOneClient,
}

/// Holds its share of the budget, and its client slots, until dropped.
pub struct BodyReservation {
    permit: OwnedSemaphorePermit,
    units: u32,
    semaphore: Arc<Semaphore>,
    clients: Vec<String>,
    in_flight: Arc<Mutex<HashMap<String, usize>>>,
}

impl BodyBudget {
    pub fn new(total_bytes: usize) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(total_bytes.div_ceil(UNIT_BYTES))), in_flight: Arc::default() }
    }

    /// `None` when that many bytes aren't free right now. Never waits, so requests don't pile up behind large ones.
    pub fn try_reserve(&self, bytes: usize) -> Option<BodyReservation> {
        let units = u32::try_from(bytes.div_ceil(UNIT_BYTES).max(1)).ok()?;
        let permit = self.semaphore.clone().try_acquire_many_owned(units).ok()?;
        Some(BodyReservation { permit, units, semaphore: self.semaphore.clone(), clients: Vec::new(), in_flight: self.in_flight.clone() })
    }

    /// Admits a body of about `expected_bytes` from `clients` (a bucket and a user id, say), if the budget could cover it and none
    /// of them has too many in flight. It holds only a small start until `BodyReservation::grow_to` is told what has arrived, so
    /// a client that declares a lot and sends nothing pins next to nothing.
    pub fn admit(&self, clients: &[String], expected_bytes: usize) -> Result<BodyReservation, Refusal> {
        if self.semaphore.available_permits() < expected_bytes.div_ceil(UNIT_BYTES) {
            return Err(Refusal::Busy);
        }
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if clients.iter().any(|client| in_flight.get(client).copied().unwrap_or(0) >= MAX_BODIES_PER_CLIENT) {
            return Err(Refusal::TooManyFromOneClient);
        }
        let mut reservation = self.try_reserve(expected_bytes.min(INITIAL_RESERVATION_BYTES)).ok_or(Refusal::Busy)?;
        for client in clients {
            *in_flight.entry(client.clone()).or_insert(0) += 1;
        }
        reservation.clients = clients.to_vec();
        Ok(reservation)
    }
}

impl BodyReservation {
    /// Makes the reservation cover `bytes`. `false` if the budget can't, in which case it keeps what it had.
    pub fn grow_to(&mut self, bytes: usize) -> bool {
        let Ok(wanted) = u32::try_from(bytes.div_ceil(UNIT_BYTES).max(1)) else { return false };
        if wanted <= self.units {
            return true;
        }
        match self.semaphore.clone().try_acquire_many_owned(wanted - self.units) {
            Ok(more) => {
                self.permit.merge(more);
                self.units = wanted;
                true
            }
            Err(_) => false,
        }
    }
}

impl Drop for BodyReservation {
    fn drop(&mut self) {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for client in &self.clients {
            if let Some(count) = in_flight.get_mut(client) {
                *count -= 1;
                if *count == 0 {
                    in_flight.remove(client);
                }
            }
        }
    }
}

impl Default for BodyBudget {
    fn default() -> Self {
        Self::new(DEFAULT_BODY_BUDGET_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_draw_down_the_budget_and_give_it_back_on_drop() {
        let budget = BodyBudget::new(10 * 1024);

        let first = budget.try_reserve(6 * 1024).expect("fits");
        assert!(budget.try_reserve(6 * 1024).is_none(), "only 4 KiB are left");
        let second = budget.try_reserve(4 * 1024).expect("exactly what is left");
        assert!(budget.try_reserve(1).is_none());

        drop(first);
        assert!(budget.try_reserve(6 * 1024).is_some());
        drop(second);
    }

    #[test]
    fn a_request_larger_than_the_whole_budget_is_never_served() {
        let budget = BodyBudget::new(1024);

        assert!(budget.try_reserve(2 * 1024).is_none());
        assert!(budget.try_reserve(usize::MAX).is_none());
    }

    fn client(name: &str) -> Vec<String> {
        vec![name.to_string()]
    }

    #[test]
    fn an_admitted_body_holds_only_what_has_arrived() {
        let budget = BodyBudget::new(10 * 1024 * 1024);

        let mut first = budget.admit(&client("a"), 8 * 1024 * 1024).expect("fits");
        let mut second = budget.admit(&client("b"), 8 * 1024 * 1024).expect("nothing has arrived yet, so nothing is pinned");

        assert!(first.grow_to(1024 * 1024));
        assert!(second.grow_to(1024 * 1024));
        assert!(budget.try_reserve(7 * 1024 * 1024).is_some(), "two stalled bodies leave room for a third request");
    }

    #[test]
    fn a_body_that_outgrows_the_budget_is_refused_and_keeps_what_it_had() {
        let budget = BodyBudget::new(128 * 1024);
        let mut body = budget.admit(&client("a"), 128 * 1024).unwrap();

        assert!(body.grow_to(96 * 1024));
        assert!(!body.grow_to(200 * 1024));
        assert!(budget.try_reserve(40 * 1024).is_none(), "the 96 KiB it had are still held");
        assert!(budget.try_reserve(32 * 1024).is_some());
    }

    #[test]
    fn admission_is_refused_when_the_budget_could_not_cover_the_declared_size() {
        let budget = BodyBudget::new(4 * 1024);

        assert_eq!(budget.admit(&client("a"), 8 * 1024).err(), Some(Refusal::Busy));
    }

    #[test]
    fn one_client_cannot_have_more_than_a_few_bodies_in_flight() {
        let budget = BodyBudget::new(1024 * 1024);
        let held: Vec<_> = (0..MAX_BODIES_PER_CLIENT).map(|_| budget.admit(&client("a"), 1024).unwrap()).collect();

        assert_eq!(budget.admit(&client("a"), 1024).err(), Some(Refusal::TooManyFromOneClient));
        assert!(budget.admit(&client("b"), 1024).is_ok(), "another client is not affected");
        let mixed = vec!["b".to_string(), "a".to_string()];
        assert_eq!(budget.admit(&mixed, 1024).err(), Some(Refusal::TooManyFromOneClient), "any one of a body's clients being at the limit is enough");

        drop(held);
        assert!(budget.admit(&client("a"), 1024).is_ok(), "the slots come back when the bodies are done");
    }
}
