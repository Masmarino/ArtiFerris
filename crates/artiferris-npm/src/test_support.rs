//! Shared helpers for the route tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::{Body, Bytes};

/// A 64 MiB request body that records whether anyone read from it.
pub fn probe_body() -> (Body, Arc<AtomicBool>) {
    let polled = Arc::new(AtomicBool::new(false));
    let flag = polled.clone();
    let mut remaining = 64;
    let stream = futures_util::stream::poll_fn(move |_| {
        flag.store(true, Ordering::SeqCst);
        if remaining == 0 {
            return std::task::Poll::Ready(None);
        }
        remaining -= 1;
        std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(Bytes::from(vec![0u8; 1024 * 1024]))))
    });
    (Body::from_stream(stream), polled)
}
