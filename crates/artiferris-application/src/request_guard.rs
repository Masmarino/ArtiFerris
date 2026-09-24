//! What the HTTP data planes share: the client's real address, the body-memory budget, body timeouts and the download dedupe.

use std::net::IpAddr;
use std::sync::Arc;

use crate::body_budget::BodyBudget;
use crate::body_read::BodyTimeouts;
use crate::client_ip::{TrustedProxies, client_ip, throttle_bucket};
use crate::download_dedupe::DownloadDedupe;

#[derive(Clone, Default)]
pub struct RequestGuard {
    pub trusted_proxies: Arc<TrustedProxies>,
    pub body_budget: BodyBudget,
    pub body_timeouts: BodyTimeouts,
    pub download_dedupe: Arc<DownloadDedupe>,
}

impl RequestGuard {
    pub fn new(trusted_proxies: Arc<TrustedProxies>) -> Self {
        Self { trusted_proxies, ..Self::default() }
    }

    /// The client's throttle bucket. `forwarded_for` is every `X-Forwarded-For` header line.
    pub fn client_bucket(&self, direct: Option<IpAddr>, forwarded_for: &[&str]) -> String {
        throttle_bucket(&client_ip(direct, forwarded_for, &self.trusted_proxies))
    }

    /// The resolved client address, as recorded in audit events.
    pub fn client_address(&self, direct: Option<IpAddr>, forwarded_for: &[&str]) -> String {
        client_ip(direct, forwarded_for, &self.trusted_proxies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bucket_follows_forwarded_for_only_behind_a_trusted_proxy() {
        let trusted = Arc::new(TrustedProxies::parse(["10.0.0.0/8"]).unwrap());
        let guard = RequestGuard::new(trusted);
        let proxy: Option<IpAddr> = "10.0.0.5".parse().ok();
        let stranger: Option<IpAddr> = "203.0.113.9".parse().ok();

        assert_eq!(guard.client_bucket(proxy, &["198.51.100.4"]), "198.51.100.4");
        assert_eq!(guard.client_bucket(stranger, &["198.51.100.4"]), "203.0.113.9");
        assert_eq!(guard.client_address(proxy, &["198.51.100.4"]), "198.51.100.4");
    }
}
