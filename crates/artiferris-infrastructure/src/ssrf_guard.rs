//! Blocks outbound requests to private and internal targets, including follow-ups a remote hands back. The host is
//! resolved once here, while the HTTP client resolves it again to connect, so a short-TTL DNS record can still rebind
//! to an internal address in between. Accepted residual risk: pinning the IP would mean a client per request or a
//! custom resolver. The identity-provider client (`oidc_http_client`) is the exception: its own resolver applies these
//! checks at connect time.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use artiferris_domain::error::DomainError;

pub async fn ensure_public_host(url: &str) -> Result<(), DomainError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| DomainError::Infrastructure(format!("invalid remote URL {url}: {e}")))?;
    let host = parsed.host_str().ok_or_else(|| DomainError::Infrastructure(format!("remote URL {url} has no host")))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    ensure_public_host_and_port(host, port).await
}

/// Same as [`ensure_public_host`] for callers (LDAP, SMTP) with a bare host and port. The same rebinding caveat
/// applies.
pub async fn ensure_public_host_and_port(host: &str, port: u16) -> Result<(), DomainError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return reject_if_private(ip, host);
    }

    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| DomainError::Infrastructure(format!("resolving host {host}: {e}")))?;
    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        reject_if_private(addr.ip(), host)?;
    }
    if !resolved_any {
        return Err(DomainError::Infrastructure(format!("host {host} did not resolve to any address")));
    }
    Ok(())
}

/// The addresses `host` resolves to, or an error if any of them is private or reserved. What an HTTP client's DNS resolver runs at
/// connect time, so a host that rebinds to an internal address after the URL was checked is refused too.
pub(crate) async fn resolve_public_addresses(host: &str) -> Result<Vec<std::net::SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let addresses: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, 0)).await?.collect();
    for address in &addresses {
        reject_if_private(address.ip(), host)?;
    }
    Ok(addresses)
}

/// A `reqwest` DNS resolver that refuses private and reserved addresses, for the clients that talk to hosts other people name.
pub(crate) struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move { Ok(Box::new(resolve_public_addresses(&host).await?.into_iter()) as reqwest::dns::Addrs) })
    }
}

pub(crate) fn reject_if_private(ip: IpAddr, host: &str) -> Result<(), DomainError> {
    if is_private_or_reserved(ip) && !crate::ssrf_allowlist::is_allowed(ip) {
        return Err(DomainError::Infrastructure(format!("host {host} resolves to a private or reserved address ({ip}), which is not allowed")));
    }
    Ok(())
}

fn is_private_or_reserved(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_or_reserved_v4(v4),
        IpAddr::V6(v6) => is_private_or_reserved_v6(v6),
    }
}

fn is_private_or_reserved_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || is_cgnat(ip)
        // 0.0.0.0/8, the whole first-octet-zero range.
        || octets[0] == 0
        // 192.0.0.0/24 (RFC 6890), distinct from the documentation range 192.0.2.0/24.
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        // 198.18.0.0/15 (RFC 2544): second octet 18 or 19.
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        // 240.0.0.0/4 (RFC 1112), which overlaps the broadcast address.
        || octets[0] >= 240
}

/// 100.64.0.0/10 (CGNAT, RFC 6598): second octet in 64..=127.
fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn is_private_or_reserved_v6(ip: Ipv6Addr) -> bool {
    // An IPv4-mapped address (::ffff:a.b.c.d) is checked against the same IPv4 ranges.
    ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.to_ipv4_mapped().is_some_and(is_private_or_reserved_v4)
        // The deprecated IPv4-compatible form (`::a.b.c.d`) differs from the mapped form and needs `to_ipv4()`.
        || ip.to_ipv4().is_some_and(is_private_or_reserved_v4)
        || is_nat64(ip)
        || is_6to4(ip)
        || is_v6_documentation(ip)
}

/// 64:ff9b::/96 and 64:ff9b:1::/48: a NAT64 gateway turns these into IPv4 traffic to the embedded address.
fn is_nat64(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    segments[0] == 0x0064 && segments[1] == 0xff9b && (segments[2..6] == [0; 4] || segments[2] == 1)
}

/// 2002::/16: the embedded IPv4 address is the tunnel endpoint.
fn is_6to4(ip: Ipv6Addr) -> bool {
    ip.segments()[0] == 0x2002
}

/// 2001:db8::/32 and 3fff::/20.
fn is_v6_documentation(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] == 0x2001 && segments[1] == 0x0db8) || (segments[0] == 0x3fff && segments[1] < 0x1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback() {
        assert!(is_private_or_reserved("127.0.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("::1".parse().unwrap()));
    }

    #[test]
    fn rejects_rfc1918_private_ranges() {
        assert!(is_private_or_reserved("10.0.0.5".parse().unwrap()));
        assert!(is_private_or_reserved("172.16.5.1".parse().unwrap()));
        assert!(is_private_or_reserved("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn rejects_link_local_including_the_cloud_metadata_address() {
        assert!(is_private_or_reserved("169.254.169.254".parse().unwrap()));
        assert!(is_private_or_reserved("fe80::1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_unique_local() {
        assert!(is_private_or_reserved("fc00::1".parse().unwrap()));
    }

    #[test]
    fn rejects_an_ipv4_mapped_private_address() {
        assert!(is_private_or_reserved("::ffff:10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn nat64_6to4_and_documentation_ipv6_ranges_are_rejected() {
        for blocked in ["64:ff9b::a00:1", "64:ff9b::7f00:1", "64:ff9b:1::1", "2002:a00:1::1", "2002:7f00:1::", "2001:db8::1", "3fff::1", "3fff:fff:ffff::1"] {
            assert!(is_private_or_reserved(blocked.parse().unwrap()), "{blocked} must be rejected");
        }
        for open in ["64:ff9a::1", "2003::1", "2001:db9::1", "3fff:1000::1"] {
            assert!(!is_private_or_reserved(open.parse().unwrap()), "{open} must stay reachable");
        }
    }

    #[test]
    fn allows_a_real_public_address() {
        assert!(!is_private_or_reserved("1.1.1.1".parse().unwrap()));
        assert!(!is_private_or_reserved("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn a_cgnat_address_is_rejected() {
        assert!(is_private_or_reserved("100.64.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("100.100.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("100.127.255.255".parse().unwrap()));
        assert!(!is_private_or_reserved("100.63.255.255".parse().unwrap()));
        assert!(!is_private_or_reserved("100.128.0.0".parse().unwrap()));
    }

    #[test]
    fn the_full_zero_slash_eight_range_is_rejected() {
        assert!(is_private_or_reserved("0.0.0.0".parse().unwrap()));
        assert!(is_private_or_reserved("0.1.2.3".parse().unwrap()));
        assert!(is_private_or_reserved("0.255.255.255".parse().unwrap()));
    }

    #[test]
    fn the_ietf_protocol_assignment_range_is_rejected() {
        assert!(is_private_or_reserved("192.0.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("192.0.0.255".parse().unwrap()));
    }

    #[test]
    fn the_documentation_range_is_still_rejected_and_is_distinct_from_ietf_protocol_assignment() {
        assert!(is_private_or_reserved("192.0.2.1".parse().unwrap()));
        assert!(!is_private_or_reserved("192.0.1.1".parse().unwrap()));
    }

    #[test]
    fn the_benchmarking_range_is_rejected() {
        assert!(is_private_or_reserved("198.18.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("198.19.255.255".parse().unwrap()));
        assert!(!is_private_or_reserved("198.17.255.255".parse().unwrap()));
        assert!(!is_private_or_reserved("198.20.0.0".parse().unwrap()));
    }

    #[test]
    fn the_reserved_240_range_is_rejected() {
        assert!(is_private_or_reserved("240.0.0.1".parse().unwrap()));
        assert!(is_private_or_reserved("250.1.2.3".parse().unwrap()));
        assert!(is_private_or_reserved("255.255.255.255".parse().unwrap()));
        assert!(!is_private_or_reserved("223.255.255.255".parse().unwrap()));
    }

    #[test]
    fn an_ipv4_compatible_ipv6_address_is_rejected() {
        assert!(is_private_or_reserved("::10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn an_ordinary_public_ip_is_still_accepted_after_widening_the_reserved_ranges() {
        assert!(!is_private_or_reserved("8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_a_url_whose_literal_host_is_a_private_ip() {
        let err = ensure_public_host("http://127.0.0.1:8080/").await.unwrap_err();
        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_a_url_pointing_at_localhost_by_name() {
        let err = ensure_public_host("http://localhost:8080/").await.unwrap_err();
        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_a_bare_host_that_is_a_private_ip() {
        let err = ensure_public_host_and_port("127.0.0.1", 389).await.unwrap_err();
        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_a_bare_hostname_resolving_to_localhost() {
        let err = ensure_public_host_and_port("localhost", 389).await.unwrap_err();
        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    #[ignore = "requires network access to resolve one.one.one.one"]
    async fn allows_a_bare_public_hostname() {
        ensure_public_host_and_port("one.one.one.one", 443).await.unwrap();
    }
}
