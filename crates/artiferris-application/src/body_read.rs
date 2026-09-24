//! Reads a request body with a size cap and time limits, so a client that trickles bytes can't hold a body-budget reservation forever.

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};

/// A body declaring a length gets that length at this rate, at least, before it is cut off.
const MIN_THROUGHPUT_BYTES_PER_SECOND: u64 = 32 * 1024;

/// The shortest overall deadline, however small the declared body.
const MIN_TOTAL: Duration = Duration::from_secs(10);

/// How long a body may go without a new chunk, and the longest a whole in-memory body may take.
#[derive(Clone, Copy, Debug)]
pub struct BodyTimeouts {
    pub idle: Duration,
    pub total: Duration,
}

impl BodyTimeouts {
    /// The overall deadline for a body that declared `declared` bytes: proportional to the size at a minimum throughput, between
    /// ten seconds and `total`. `total` when no length was declared.
    pub fn total_for(&self, declared: Option<usize>) -> Duration {
        let Some(declared) = declared else { return self.total };
        Duration::from_secs(declared as u64 / MIN_THROUGHPUT_BYTES_PER_SECOND).max(MIN_TOTAL).min(self.total)
    }
}

impl Default for BodyTimeouts {
    fn default() -> Self {
        Self { idle: Duration::from_secs(30), total: Duration::from_secs(120) }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BodyReadError {
    TooLarge,
    TimedOut,
    Unreadable,
    /// `charge` refused to cover what has arrived.
    Busy,
}

/// The whole body, at most `limit` bytes, with no more than `timeouts.idle` between chunks and `timeouts.total_for(declared)` overall.
/// `charge` is told the size the body would have with the next chunk in, before that chunk is kept, and may refuse it.
pub async fn read_limited<S, E>(mut body: S, limit: usize, timeouts: BodyTimeouts, declared: Option<usize>, mut charge: impl FnMut(usize) -> bool) -> Result<Bytes, BodyReadError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    let read = async {
        let mut buffer = BytesMut::new();
        loop {
            match tokio::time::timeout(timeouts.idle, body.next()).await {
                Err(_) => return Err(BodyReadError::TimedOut),
                Ok(None) => return Ok(buffer.freeze()),
                Ok(Some(Err(_))) => return Err(BodyReadError::Unreadable),
                Ok(Some(Ok(chunk))) => {
                    let received = buffer.len() + chunk.len();
                    if received > limit {
                        return Err(BodyReadError::TooLarge);
                    }
                    if !charge(received) {
                        return Err(BodyReadError::Busy);
                    }
                    buffer.extend_from_slice(&chunk);
                }
            }
        }
    };
    tokio::time::timeout(timeouts.total_for(declared), read).await.unwrap_or(Err(BodyReadError::TimedOut))
}

/// Passes `body` through, failing with `on_timeout()` (and ending) if no chunk arrives within `idle`.
pub fn with_idle_timeout<S, T, E>(body: S, idle: Duration, on_timeout: fn() -> E) -> impl Stream<Item = Result<T, E>>
where
    S: Stream<Item = Result<T, E>> + Unpin,
{
    futures::stream::unfold(Some(body), move |state| async move {
        let mut body = state?;
        match tokio::time::timeout(idle, body.next()).await {
            Ok(Some(item)) => Some((item, Some(body))),
            Ok(None) => None,
            Err(_) => Some((Err(on_timeout()), None)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    fn timeouts(idle_ms: u64, total_ms: u64) -> BodyTimeouts {
        BodyTimeouts { idle: Duration::from_millis(idle_ms), total: Duration::from_millis(total_ms) }
    }

    fn chunks(parts: Vec<&'static [u8]>) -> impl Stream<Item = Result<Bytes, Infallible>> + Unpin {
        futures::stream::iter(parts.into_iter().map(|part| Ok(Bytes::from_static(part))))
    }

    #[tokio::test]
    async fn a_body_within_the_limit_is_read_whole() {
        let body = read_limited(chunks(vec![b"ab", b"cd"]), 10, timeouts(100, 1000), None, |_| true).await.unwrap();
        assert_eq!(body, Bytes::from_static(b"abcd"));
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_refused() {
        assert_eq!(read_limited(chunks(vec![b"abc", b"def"]), 5, timeouts(100, 1000), None, |_| true).await, Err(BodyReadError::TooLarge));
    }

    #[tokio::test]
    async fn a_body_that_stalls_times_out() {
        let stalled = futures::stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"a"))]).chain(futures::stream::pending());
        assert_eq!(read_limited(Box::pin(stalled), 10, timeouts(50, 1000), None, |_| true).await, Err(BodyReadError::TimedOut));
    }

    #[tokio::test]
    async fn a_body_that_trickles_past_the_total_deadline_times_out() {
        let trickle = futures::stream::unfold(0u8, |n| async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Some((Ok::<_, Infallible>(Bytes::from_static(b"x")), n + 1))
        });
        assert_eq!(read_limited(Box::pin(trickle), 1000, timeouts(100, 150), None, |_| true).await, Err(BodyReadError::TimedOut));
    }

    #[tokio::test]
    async fn a_body_the_charge_refuses_is_dropped_as_busy() {
        assert_eq!(read_limited(chunks(vec![b"abc", b"def"]), 100, timeouts(100, 1000), None, |received| received <= 3).await, Err(BodyReadError::Busy));
    }

    #[test]
    fn the_deadline_follows_the_declared_size_between_ten_seconds_and_the_total() {
        let timeouts = BodyTimeouts::default();

        assert_eq!(timeouts.total_for(None), Duration::from_secs(120));
        assert_eq!(timeouts.total_for(Some(1024)), Duration::from_secs(10), "a small body still gets ten seconds");
        assert_eq!(timeouts.total_for(Some(32 * 1024 * 60)), Duration::from_secs(60), "32 KiB/s for a 1.9 MiB body");
        assert_eq!(timeouts.total_for(Some(500 * 1024 * 1024)), Duration::from_secs(120), "never longer than the total");
    }

    #[tokio::test]
    async fn a_stalled_stream_ends_with_the_timeout_error() {
        let stalled = futures::stream::iter([Ok::<_, &'static str>(1u8)]).chain(futures::stream::pending());
        let mut wrapped = Box::pin(with_idle_timeout(Box::pin(stalled), Duration::from_millis(50), || "stalled"));

        assert_eq!(wrapped.next().await, Some(Ok(1)));
        assert_eq!(wrapped.next().await, Some(Err("stalled")));
        assert_eq!(wrapped.next().await, None);
    }
}
