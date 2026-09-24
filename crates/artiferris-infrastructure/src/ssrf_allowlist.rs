//! Operator allowlist for the SSRF guard: `ARTIFERRIS_SSRF_ALLOWED_CIDRS` lists private ranges
//! (an on-prem directory, an internal relay) that admin-configured hosts may resolve to.
//! Everything not listed stays blocked, and only the operator can list anything.

use std::net::IpAddr;
use std::sync::OnceLock;

use artiferris_application::client_ip::TrustedProxies;

static ALLOWED: OnceLock<TrustedProxies> = OnceLock::new();

/// Comma-separated addresses or CIDR ranges. A typo is an error rather than an empty list, so a
/// mistyped range never silently keeps an internal host blocked or, worse, allows more than meant.
pub fn parse_allowed_cidrs(raw: &str) -> Result<TrustedProxies, String> {
    TrustedProxies::parse(raw.split(','))
}

/// Called once at startup; later calls are ignored.
pub fn configure(allowed: TrustedProxies) {
    let _ = ALLOWED.set(allowed);
}

pub(crate) fn is_allowed(ip: IpAddr) -> bool {
    ALLOWED.get().is_some_and(|allowed| allowed.contains(ip))
}

/// True when the host resolves, and every address it resolves to is in the allowlist.
pub async fn host_is_allow_listed(host: &str, port: u16) -> bool {
    let addresses: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => match tokio::net::lookup_host((host, port)).await {
            Ok(resolved) => resolved.map(|a| a.ip()).collect(),
            Err(_) => return false,
        },
    };
    !addresses.is_empty() && addresses.into_iter().all(is_allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_single_addresses_parse_and_a_typo_is_an_error() {
        let allowed = parse_allowed_cidrs("10.0.0.0/8, 192.168.1.5,fd00::/8").unwrap();
        assert!(allowed.contains("10.20.30.40".parse().unwrap()));
        assert!(allowed.contains("192.168.1.5".parse().unwrap()));
        assert!(!allowed.contains("192.168.1.6".parse().unwrap()));
        assert!(parse_allowed_cidrs("10.0.0.0/8,oops").is_err());
        assert!(parse_allowed_cidrs("10.0.0.0/40").is_err());
        assert!(parse_allowed_cidrs("0.0.0.0/0").is_err(), "a /0 would switch the guard off");
        assert!(parse_allowed_cidrs("::/0").is_err());
        assert!(parse_allowed_cidrs("").unwrap().is_empty());
    }

    /// One test owns the process-wide list, so nothing else in this binary depends on it being unset.
    #[tokio::test]
    async fn only_listed_private_ranges_pass_the_guard_once_configured() {
        configure(parse_allowed_cidrs("10.99.0.0/16").unwrap());

        crate::ssrf_guard::ensure_public_host_and_port("10.99.1.2", 389).await.expect("listed range passes");
        crate::ssrf_guard::ensure_public_host("ldap://10.99.1.2:389").await.expect("listed range passes for URLs too");
        assert!(crate::ssrf_guard::ensure_public_host_and_port("10.98.1.2", 389).await.is_err(), "a neighbouring range stays blocked");
        assert!(crate::ssrf_guard::ensure_public_host_and_port("169.254.169.254", 80).await.is_err(), "the metadata address stays blocked");
        assert!(crate::ssrf_guard::ensure_public_host_and_port("127.0.0.1", 80).await.is_err());

        assert!(host_is_allow_listed("10.99.1.2", 25).await);
        assert!(!host_is_allow_listed("10.98.1.2", 25).await);
        assert!(!host_is_allow_listed("8.8.8.8", 25).await, "a public address is not 'allow-listed' just because it passes the guard");
    }
}
