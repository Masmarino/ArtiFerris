//! Which address a request really came from, for throttling and audit trails.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The reverse proxies whose `X-Forwarded-For` we believe: single addresses or CIDR ranges (`10.0.0.4`, `10.0.0.0/8`, `fd00::/8`).
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    ranges: Vec<(IpAddr, u8)>,
}

impl TrustedProxies {
    /// Entries that don't parse are returned as errors rather than skipped: a typo must not silently mean "trust nobody".
    pub fn parse<'a>(entries: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for entry in entries.into_iter().map(str::trim).filter(|e| !e.is_empty()) {
            let (address, prefix) = match entry.split_once('/') {
                Some((address, prefix)) => (address, Some(prefix)),
                None => (entry, None),
            };
            let address: IpAddr = address.parse().map_err(|_| format!("`{entry}` is not an IP address or CIDR range"))?;
            let max = if address.is_ipv4() { 32 } else { 128 };
            let prefix = match prefix {
                Some(prefix) => prefix.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("`{entry}` has an invalid prefix length"))?,
                None => max,
            };
            if prefix == 0 {
                return Err(format!("`{entry}` matches every address on the internet; list the ranges you mean"));
            }
            ranges.push((address, prefix));
        }
        Ok(Self { ranges })
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.ranges.iter().any(|(network, prefix)| same_prefix(*network, ip, *prefix))
    }
}

fn same_prefix(network: IpAddr, ip: IpAddr, prefix: u8) -> bool {
    match (network, ip) {
        (IpAddr::V4(network), IpAddr::V4(ip)) => masked(u128::from(u32::from(network)), u128::from(u32::from(ip)), 32, prefix),
        (IpAddr::V6(network), IpAddr::V6(ip)) => masked(u128::from(network), u128::from(ip), 128, prefix),
        // An IPv4-mapped IPv6 peer (`::ffff:10.0.0.4`) is that IPv4 address.
        (IpAddr::V4(_), IpAddr::V6(ip)) => ip.to_ipv4_mapped().is_some_and(|mapped| same_prefix(network, IpAddr::V4(mapped), prefix)),
        (IpAddr::V6(_), IpAddr::V4(_)) => false,
    }
}

fn masked(network: u128, ip: u128, bits: u32, prefix: u8) -> bool {
    let shift = bits - u32::from(prefix);
    (network >> shift) == (ip >> shift)
}

/// Lets a condition through at most once per interval.
struct RateLimitedWarning {
    last_seconds: AtomicU64,
}

const WARNING_INTERVAL_SECONDS: u64 = 60 * 60;

impl RateLimitedWarning {
    const fn new() -> Self {
        Self { last_seconds: AtomicU64::new(0) }
    }

    fn should_warn(&self, now_seconds: u64) -> bool {
        let last = self.last_seconds.load(Ordering::Relaxed);
        (last == 0 || now_seconds.saturating_sub(last) >= WARNING_INTERVAL_SECONDS) && self.last_seconds.compare_exchange(last, now_seconds, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    }
}

static MISCONFIGURED_PROXY_WARNING: RateLimitedWarning = RateLimitedWarning::new();

fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private_or_loopback(IpAddr::V4(v4)),
            None => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        },
    }
}

/// A private peer forwarding a client address is most likely a proxy nobody configured.
fn forwarded_from_untrusted_private_peer(direct: IpAddr, forwarded_for: &[&str], trusted: &TrustedProxies) -> bool {
    trusted.is_empty() && is_private_or_loopback(direct) && forwarded_for.iter().any(|line| !line.trim().is_empty())
}

fn warn_about_untrusted_proxy(now_seconds: u64) {
    if MISCONFIGURED_PROXY_WARNING.should_warn(now_seconds) {
        tracing::warn!("requests carry X-Forwarded-For from a private peer but TRUSTED_PROXY_IPS is empty: every client shares one throttle bucket");
    }
}

/// A hop as a proxy may write it: a bare address, `ip:port`, `[v6]` or `[v6]:port`. Anything else (`unknown`, an obfuscated token, empty) is `None`.
fn parse_hop(entry: &str) -> Option<IpAddr> {
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(addr) = entry.parse::<SocketAddr>() {
        return Some(addr.ip());
    }
    entry.strip_prefix('[')?.strip_suffix(']')?.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
}

/// The client behind `direct`. `forwarded_for` is every `X-Forwarded-For` header line. It is read only when `direct` is a trusted
/// proxy, right to left (the left end is whatever the client claimed), skipping trusted hops. The first hop that doesn't parse ends
/// the scan at `direct`: whatever sits to its left was written by someone we can't vouch for.
pub fn client_ip(direct: Option<IpAddr>, forwarded_for: &[&str], trusted: &TrustedProxies) -> String {
    let Some(direct) = direct else { return "unknown".to_string() };
    if forwarded_from_untrusted_private_peer(direct, forwarded_for, trusted) {
        warn_about_untrusted_proxy(SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |elapsed| elapsed.as_secs().max(1)));
    }
    if trusted.contains(direct) {
        let hops = forwarded_for.iter().filter(|line| !line.trim().is_empty()).flat_map(|line| line.split(',')).map(str::trim).rev();
        for hop in hops {
            match parse_hop(hop) {
                Some(ip) if trusted.contains(ip) => continue,
                Some(ip) => return ip.to_string(),
                None => break,
            }
        }
    }
    direct.to_string()
}

/// The key a throttle should count under: an IPv6 client is a whole /64 (one subscriber owns that many addresses), IPv4 stays as is.
pub fn throttle_bucket(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => format!("{:x}/64", u128::from(v6) >> 64),
        },
        _ => ip.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted(entries: &[&str]) -> TrustedProxies {
        TrustedProxies::parse(entries.iter().copied()).unwrap()
    }

    fn ip(text: &str) -> Option<IpAddr> {
        Some(text.parse().unwrap())
    }

    #[test]
    fn a_forwarded_header_from_a_private_peer_with_no_trusted_proxies_is_the_misconfiguration_to_warn_about() {
        let none = trusted(&[]);
        let some = trusted(&["10.0.0.0/8"]);
        let private = |peer: &str| forwarded_from_untrusted_private_peer(peer.parse().unwrap(), &["203.0.113.9"], &none);

        assert!(private("10.42.0.7") && private("192.168.1.2") && private("172.16.0.1") && private("127.0.0.1") && private("::1") && private("fd12::1") && private("::ffff:10.1.1.1"));
        assert!(!private("203.0.113.9"), "a public peer forwarding a header is just a client lying");
        assert!(!forwarded_from_untrusted_private_peer("10.42.0.7".parse().unwrap(), &["203.0.113.9"], &some), "configured, nothing to warn about");
        assert!(!forwarded_from_untrusted_private_peer("10.42.0.7".parse().unwrap(), &[], &none), "no header, no proxy in sight");
        assert!(!forwarded_from_untrusted_private_peer("10.42.0.7".parse().unwrap(), &["  "], &none));
    }

    #[test]
    fn the_warning_fires_once_per_hour() {
        let warning = RateLimitedWarning::new();

        assert!(warning.should_warn(1_000));
        assert!(!warning.should_warn(1_001));
        assert!(!warning.should_warn(1_000 + WARNING_INTERVAL_SECONDS - 1));
        assert!(warning.should_warn(1_000 + WARNING_INTERVAL_SECONDS));
        assert!(!warning.should_warn(1_000 + WARNING_INTERVAL_SECONDS + 5));
    }

    #[test]
    fn resolving_a_client_ip_still_ignores_the_header_when_nobody_is_trusted() {
        assert_eq!(client_ip(ip("10.42.0.7"), &["203.0.113.9"], &trusted(&[])), "10.42.0.7");
    }

    #[test]
    fn a_cidr_range_and_a_single_address_are_both_trusted() {
        let proxies = trusted(&["10.0.0.0/8", "192.168.1.5", "fd00::/8"]);

        for trusted_ip in ["10.1.2.3", "192.168.1.5", "fd12::1", "::ffff:10.9.9.9"] {
            assert!(proxies.contains(trusted_ip.parse().unwrap()), "{trusted_ip}");
        }
        for other in ["11.0.0.1", "192.168.1.6", "2001:db8::1"] {
            assert!(!proxies.contains(other.parse().unwrap()), "{other}");
        }
    }

    #[test]
    fn a_slash_zero_range_is_refused_because_it_matches_the_whole_internet() {
        for whole_internet in ["0.0.0.0/0", "::/0", "10.0.0.0/8, 0.0.0.0/0"] {
            let error = TrustedProxies::parse(whole_internet.split(',')).unwrap_err();
            assert!(error.contains("every address"), "{whole_internet}: {error}");
        }
        assert!(TrustedProxies::parse(["0.0.0.0/1"]).is_ok());
    }

    #[test]
    fn a_typo_is_an_error_not_silence() {
        for bad in ["10.0.0.0/33", "not-an-ip", "10.0.0.0/x", "fd00::/129"] {
            assert!(TrustedProxies::parse([bad]).is_err(), "{bad}");
        }
        assert!(TrustedProxies::parse(["", "  "]).unwrap().is_empty());
    }

    #[test]
    fn forwarded_for_is_ignored_unless_the_direct_peer_is_a_trusted_proxy() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(client_ip(ip("203.0.113.7"), &["1.2.3.4"], &proxies), "203.0.113.7", "a client cannot pick its own address");
        assert_eq!(client_ip(ip("10.0.0.5"), &["198.51.100.4"], &proxies), "198.51.100.4");
    }

    #[test]
    fn the_right_most_untrusted_hop_is_the_client_and_the_left_end_is_never_believed() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, 198.51.100.4, 10.0.0.9"], &proxies), "198.51.100.4");
    }

    #[test]
    fn every_forwarded_for_header_line_counts_not_just_the_first() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6", "198.51.100.4"], &proxies), "198.51.100.4", "a proxy that appends its own header line must not hand the client the decision");
    }

    #[test]
    fn a_hop_written_as_ip_with_port_or_bracketed_v6_is_read_as_that_address() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, 198.51.100.4:5555"], &proxies), "198.51.100.4");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, [2001:db8::7]:443"], &proxies), "2001:db8::7");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, [2001:db8::7]"], &proxies), "2001:db8::7");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, 2001:db8::7"], &proxies), "2001:db8::7");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, [::1]:80"], &proxies), "::1");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6, 198.51.100.4, 10.0.0.9:80"], &proxies), "198.51.100.4", "a trusted hop with a port is still skipped");
    }

    #[test]
    fn an_unparseable_hop_ends_the_scan_at_the_direct_peer_instead_of_trusting_what_is_left_of_it() {
        let proxies = trusted(&["10.0.0.0/8"]);

        for chain in ["6.6.6.6, unknown", "6.6.6.6, ", "198.51.100.4, , 10.0.0.9", "6.6.6.6, 1.2.3.4:x", "6.6.6.6, [::1", "6.6.6.6, 1.2.3.4:80:80", "6.6.6.6, unknown, 10.0.0.9"] {
            assert_eq!(client_ip(ip("10.0.0.5"), &[chain], &proxies), "10.0.0.5", "{chain}");
        }
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6", "unknown"], &proxies), "10.0.0.5", "across header lines too");
        assert_eq!(client_ip(ip("10.0.0.5"), &["6.6.6.6", ""], &proxies), "6.6.6.6", "an empty header line has no hops");
    }

    #[test]
    fn a_garbled_or_all_trusted_chain_falls_back_to_the_direct_peer() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(client_ip(ip("10.0.0.5"), &["not-an-ip"], &proxies), "10.0.0.5");
        assert_eq!(client_ip(ip("10.0.0.5"), &["10.0.0.7, 10.0.0.8"], &proxies), "10.0.0.5");
        assert_eq!(client_ip(None, &["1.2.3.4"], &proxies), "unknown");
    }

    #[test]
    fn an_ipv6_client_is_throttled_as_its_whole_slash_64() {
        assert_eq!(throttle_bucket("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), throttle_bucket("2001:db8:1:2::1"));
        assert_ne!(throttle_bucket("2001:db8:1:2::1"), throttle_bucket("2001:db8:1:3::1"));
        assert_eq!(throttle_bucket("203.0.113.7"), "203.0.113.7");
        assert_eq!(throttle_bucket("::ffff:203.0.113.7"), "203.0.113.7");
        assert_eq!(throttle_bucket("unknown"), "unknown");
    }
}
