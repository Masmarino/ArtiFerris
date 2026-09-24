/// `localhost` or any `*.localhost` name (this repo's `artiferris.localhost` dev domain), with or without a port. Not lookalikes such as `localhost.evil.com`.
pub fn is_local_dev_domain(domain: &str) -> bool {
    let host = domain.split(':').next().unwrap_or(domain).to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// `http` for a local dev domain, `https` for everything else.
pub fn scheme_for_domain(domain: &str) -> &'static str {
    if is_local_dev_domain(domain) { "http" } else { "https" }
}

/// `host` unchanged when it is a plain `host[:port]` that is `base_domain` or one of its subdomains. Anything with
/// other characters (`/ # ? @ \`, spaces, control characters), an empty or malformed label or a non-numeric port is `None`.
pub fn own_host<'a>(host: &'a str, base_domain: &str) -> Option<&'a str> {
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    if port.is_some_and(|port| port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let well_formed = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-') && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !well_formed {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let is_ours = name == base_domain || name.strip_suffix(base_domain).is_some_and(|prefix| prefix.ends_with('.'));
    is_ours.then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_and_its_subdomains_are_local_dev_domains() {
        for domain in ["localhost", "localhost:4200", "artiferris.localhost", "app.artiferris.localhost:8080", "LOCALHOST"] {
            assert!(is_local_dev_domain(domain), "{domain}");
        }
    }

    #[test]
    fn lookalikes_are_not_local_dev_domains() {
        for domain in ["localhost.evil.com", "notlocalhost", "artiferris.example.com", "evil-localhost", "localhostx:80", ""] {
            assert!(!is_local_dev_domain(domain), "{domain}");
        }
    }

    #[test]
    fn dev_domains_use_http_and_everything_else_https() {
        assert_eq!(scheme_for_domain("artiferris.localhost"), "http");
        assert_eq!(scheme_for_domain("artiferris.example.com"), "https");
    }

    #[test]
    fn own_host_accepts_the_base_domain_and_its_subdomains_with_an_optional_port() {
        for host in ["artiferris.pro", "acme.artiferris.pro", "Acme.Artiferris.pro:8080", "a.b.artiferris.pro:443"] {
            assert_eq!(own_host(host, "artiferris.pro"), Some(host), "{host}");
        }
    }

    #[test]
    fn own_host_rejects_anything_that_is_not_a_bare_host_of_the_base_domain() {
        for host in [
            "evil.example",
            "artiferris.pro.evil.example",
            "notartiferris.pro",
            ".artiferris.pro",
            "evil.example/x.artiferris.pro",
            "evil.example#.artiferris.pro",
            "evil.example?.artiferris.pro",
            "a@evil.example/.artiferris.pro",
            "a@evil.artiferris.pro",
            "evil\\.artiferris.pro",
            "evil .artiferris.pro",
            "evil\t.artiferris.pro",
            "evil\0.artiferris.pro",
            "acme.artiferris.pro:80/../x",
            "acme.artiferris.pro:",
            "acme.artiferris.pro:http",
            "acme.artiferris.pro:1234567",
            "-a.artiferris.pro",
            "a..artiferris.pro",
            "",
        ] {
            assert_eq!(own_host(host, "artiferris.pro"), None, "{host:?}");
        }
    }
}
