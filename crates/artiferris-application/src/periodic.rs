//! Runs a job on a timer on every instance, but only once per interval across all of them: each tick asks the shared
//! store whether the job is due, and only the instance that is told so runs it.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use artiferris_domain::periodic_job::PeriodicJobPort;

/// Waits `first_delay`, then at every `interval` runs `job` if this instance wins the claim for `name`. A store that
/// does not answer skips the round and is logged: better a sweep that waits than every instance running it at once.
pub async fn run_claimed_forever<F, Fut>(store: Arc<dyn PeriodicJobPort>, name: &'static str, first_delay: Duration, interval: Duration, mut job: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    tokio::time::sleep(first_delay).await;
    loop {
        match store.claim(name, interval).await {
            Ok(true) => job().await,
            Ok(false) => tracing::debug!(job = name, "another instance ran this job recently"),
            Err(e) => tracing::warn!(job = name, "could not tell whether this job is due, skipping this round: {e}"),
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::error::DomainError;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scripted {
        answers: std::sync::Mutex<Vec<Result<bool, DomainError>>>,
        asked: AtomicUsize,
    }

    #[async_trait]
    impl PeriodicJobPort for Scripted {
        async fn claim(&self, _name: &str, _interval: Duration) -> Result<bool, DomainError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() { Ok(false) } else { answers.remove(0) }
        }
    }

    #[tokio::test]
    async fn the_job_runs_only_on_the_rounds_this_instance_wins() {
        let store = Arc::new(Scripted { answers: std::sync::Mutex::new(vec![Ok(true), Ok(false), Err(DomainError::Infrastructure("down".into())), Ok(true)]), asked: AtomicUsize::new(0) });
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();

        let task = tokio::spawn(run_claimed_forever(store.clone(), "test", Duration::ZERO, Duration::from_millis(10), move || {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
            }
        }));
        tokio::time::sleep(Duration::from_millis(150)).await;
        task.abort();

        assert_eq!(runs.load(Ordering::SeqCst), 2, "won twice out of the four scripted rounds");
        assert!(store.asked.load(Ordering::SeqCst) >= 4);
    }

    #[tokio::test]
    async fn nothing_runs_before_the_first_delay() {
        let store = Arc::new(Scripted { answers: std::sync::Mutex::new(vec![Ok(true)]), asked: AtomicUsize::new(0) });

        let task = tokio::spawn(run_claimed_forever(store.clone(), "test", Duration::from_secs(60), Duration::from_secs(60), || async {}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        task.abort();

        assert_eq!(store.asked.load(Ordering::SeqCst), 0);
    }
}
