pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    /// Encrypts secrets at rest (SMTP, LDAP, OIDC, TOTP). Separate from `jwt_secret` so a leaked JWT_SECRET does not
    /// expose them. Mandatory, no fallback.
    pub secrets_encryption_key: String,
    pub storage_root: String,
    pub bind_addr: String,
    /// `None` restricts CORS to localhost/127.0.0.1/[::1] on any port (see `main::cors_layer`).
    pub cors_allowed_origin: Option<String>,
    /// `None` (the default) derives the Docker Bearer-challenge realm per request from `Host` and `public_url`'s scheme. Set `ARTIFERRIS_DOCKER_TOKEN_REALM` only if that request info can't be trusted.
    pub docker_token_realm_override: Option<String>,
    /// Base URL invitation links are built against. Defaults from `bind_addr`, override with `PUBLIC_URL` behind a proxy.
    pub public_url: String,
    /// Postgres pool size, override with `DB_MAX_CONNECTIONS`.
    pub db_max_connections: u32,
    /// The base domain subdomains are resolved against. Mandatory (`ARTIFERRIS_BASE_DOMAIN`), no fallback — a misconfigured deployment must fail loudly at startup.
    pub artiferris_base_domain: String,
    /// Direct TCP peers allowed to set `X-Forwarded-For` (`TRUSTED_PROXY_IPS`, comma-separated). Empty by default — a client behind no configured reverse proxy can't spoof its throttle key.
    pub trusted_proxy_ips: std::collections::HashSet<String>,
    /// How long security and administrative audit events are kept (`AUDIT_RETENTION_DAYS`, default 365, `0` keeps everything). `None` disables the sweep.
    pub audit_retention_days: Option<i64>,
}

/// Single addresses or CIDR ranges, comma-separated. A typo fails startup: silently trusting nobody would put every client behind the proxy in one throttle bucket.
fn trusted_proxy_entries(raw: Option<&str>) -> std::collections::HashSet<String> {
    let entries: std::collections::HashSet<String> = raw.map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    if let Err(e) = artiferris_application::client_ip::TrustedProxies::parse(entries.iter().map(String::as_str)) {
        panic!("invalid TRUSTED_PROXY_IPS: {e}");
    }
    entries
}

/// Unset or empty keeps the default; anything else that is not a positive number is a typo, not a reason to guess.
fn parsed_db_max_connections(raw: Option<&str>) -> Result<u32, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(artiferris_infrastructure::postgres::DEFAULT_DB_MAX_CONNECTIONS),
        Some(value) => value.parse::<u32>().ok().filter(|connections| *connections > 0).ok_or_else(|| format!("DB_MAX_CONNECTIONS must be a positive number, got `{value}`")),
    }
}

const MIN_SECRET_BYTES: usize = 32;

/// Rejects short values and the `change-me…` placeholders from `.env.example`, which are public.
fn strong_secret(name: &str, value: &str) -> Result<(), String> {
    if value.len() < MIN_SECRET_BYTES {
        return Err(format!("{name} must be at least {MIN_SECRET_BYTES} bytes; generate one with `openssl rand -base64 32`"));
    }
    if value.to_ascii_lowercase().starts_with("change-me") {
        return Err(format!("{name} is still the `change-me` placeholder from .env.example; generate a real value with `openssl rand -base64 32`"));
    }
    Ok(())
}

fn validated_jwt_secret(raw: Option<String>) -> Result<String, String> {
    let secret = raw.filter(|s| !s.is_empty()).ok_or("JWT_SECRET must be set")?;
    strong_secret("JWT_SECRET", &secret)?;
    Ok(secret)
}

/// Reusing JWT_SECRET here would let one leaked secret both forge sessions and decrypt every stored secret, so it is
/// refused at startup. Kept out of `from_env` so the rule is testable without process-global env vars.
fn validated_secrets_encryption_key(raw: Option<String>, jwt_secret: &str) -> Result<String, String> {
    let key = raw
        .filter(|s| !s.is_empty())
        .ok_or("SECRETS_ENCRYPTION_KEY must be set to a value distinct from JWT_SECRET, there is no fallback; generate one with `openssl rand -base64 32`")?;
    strong_secret("SECRETS_ENCRYPTION_KEY", &key)?;
    if key == jwt_secret {
        return Err("SECRETS_ENCRYPTION_KEY must be a DIFFERENT value from JWT_SECRET: sharing one value means a leaked JWT_SECRET also decrypts every stored secret".to_string());
    }
    Ok(key)
}

/// Unset or empty means no bootstrap admin, which is fine; the published placeholder is not.
fn checked_bootstrap_admin_password(raw: Option<&str>) -> Result<(), String> {
    match raw {
        Some(password) if password.to_ascii_lowercase().starts_with("change-me") => {
            Err("ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD is still the `change-me` placeholder from .env.example; set a real password or unset it".to_string())
        }
        _ => Ok(()),
    }
}

/// A public URL whose host is the bind-all address would put `http://0.0.0.0:8080` in the docker realm, HSTS and passkey origin of a real deployment.
fn validated_public_url(public_url: String, base_domain: &str) -> Result<String, String> {
    let authority = public_url.split_once("://").map_or(public_url.as_str(), |(_, rest)| rest).split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = if host_port.starts_with('[') { host_port.split_once(']').map_or(host_port, |(h, _)| h.trim_start_matches('[')) } else { host_port.split(':').next().unwrap_or_default() };
    if matches!(host, "0.0.0.0" | "::") && !artiferris_application::base_domain::is_local_dev_domain(base_domain) {
        return Err(format!("PUBLIC_URL resolves to {public_url}, which no client can reach; set PUBLIC_URL to this deployment's external URL (e.g. https://app.{base_domain})"));
    }
    Ok(public_url)
}

impl Config {
    pub fn from_env() -> Self {
        let jwt_secret = validated_jwt_secret(std::env::var("JWT_SECRET").ok()).unwrap_or_else(|message| panic!("{message}"));
        let artiferris_base_domain = std::env::var("ARTIFERRIS_BASE_DOMAIN").expect("ARTIFERRIS_BASE_DOMAIN must be set").to_ascii_lowercase();
        let public_url = std::env::var("PUBLIC_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("http://{}", std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string())));
        checked_bootstrap_admin_password(std::env::var("ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD").ok().as_deref()).unwrap_or_else(|message| panic!("{message}"));
        Self {
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            secrets_encryption_key: validated_secrets_encryption_key(std::env::var("SECRETS_ENCRYPTION_KEY").ok(), &jwt_secret).unwrap_or_else(|message| panic!("{message}")),
            jwt_secret,
            storage_root: std::env::var("STORAGE_ROOT").unwrap_or_else(|_| "./data".to_string()),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            // docker-compose ${VAR:-} passthroughs set an empty string, not unset.
            cors_allowed_origin: std::env::var("CORS_ALLOWED_ORIGIN").ok().filter(|s| !s.is_empty()),
            docker_token_realm_override: std::env::var("ARTIFERRIS_DOCKER_TOKEN_REALM").ok().filter(|s| !s.is_empty()),
            public_url: validated_public_url(public_url, &artiferris_base_domain).unwrap_or_else(|message| panic!("{message}")),
            db_max_connections: parsed_db_max_connections(std::env::var("DB_MAX_CONNECTIONS").ok().as_deref()).unwrap_or_else(|message| panic!("{message}")),
            artiferris_base_domain,
            trusted_proxy_ips: trusted_proxy_entries(std::env::var("TRUSTED_PROXY_IPS").ok().as_deref()),
            audit_retention_days: artiferris_application::audit_retention::parse_audit_retention_days(std::env::var("AUDIT_RETENTION_DAYS").ok().as_deref()).unwrap_or_else(|message| panic!("{message}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JWT: &str = "jwt-secret-jwt-secret-jwt-secret-jwt-secret";
    const KEY: &str = "its-own-random-value-its-own-random-value";

    /// Driving the rules directly, not `from_env` — parallel tests would race over `set_var`.
    #[test]
    fn the_pool_size_defaults_when_unset_and_refuses_a_typo() {
        assert_eq!(parsed_db_max_connections(None), Ok(artiferris_infrastructure::postgres::DEFAULT_DB_MAX_CONNECTIONS));
        assert_eq!(parsed_db_max_connections(Some("")), Ok(artiferris_infrastructure::postgres::DEFAULT_DB_MAX_CONNECTIONS));
        assert_eq!(parsed_db_max_connections(Some(" 25 ")), Ok(25));
        for bad in ["ten", "0", "-1", "10.5"] {
            assert!(parsed_db_max_connections(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    #[should_panic(expected = "invalid TRUSTED_PROXY_IPS")]
    fn trusting_every_address_stops_startup() {
        trusted_proxy_entries(Some("10.0.0.0/8,0.0.0.0/0"));
    }

    #[test]
    fn a_secrets_encryption_key_distinct_from_the_jwt_secret_is_accepted() {
        assert_eq!(validated_secrets_encryption_key(Some(KEY.to_string()), JWT).unwrap(), KEY);
    }

    #[test]
    fn a_secrets_encryption_key_equal_to_the_jwt_secret_is_rejected() {
        let error = validated_secrets_encryption_key(Some(JWT.to_string()), JWT).unwrap_err();
        assert!(error.contains("DIFFERENT value from JWT_SECRET"), "got: {error}");
    }

    #[test]
    fn a_missing_or_empty_secrets_encryption_key_is_still_rejected() {
        assert!(validated_secrets_encryption_key(None, JWT).is_err());
        assert!(validated_secrets_encryption_key(Some(String::new()), JWT).is_err());
    }

    #[test]
    fn short_secrets_are_rejected() {
        assert!(validated_jwt_secret(Some("a".to_string())).unwrap_err().contains("at least 32 bytes"));
        assert!(validated_jwt_secret(Some("x".repeat(31))).is_err());
        assert!(validated_jwt_secret(Some("x".repeat(32))).is_ok());
        assert!(validated_secrets_encryption_key(Some("b".to_string()), JWT).unwrap_err().contains("at least 32 bytes"));
    }

    #[test]
    fn the_env_example_placeholders_are_rejected_even_when_long_enough() {
        for placeholder in ["change-me-to-a-long-random-string", "change-me-to-a-different-long-random-string", "CHANGE-ME-to-a-long-random-string-again"] {
            assert!(validated_jwt_secret(Some(placeholder.to_string())).unwrap_err().contains("placeholder"), "{placeholder}");
            assert!(validated_secrets_encryption_key(Some(placeholder.to_string()), JWT).unwrap_err().contains("placeholder"), "{placeholder}");
        }
    }

    #[test]
    fn a_missing_jwt_secret_is_rejected() {
        assert!(validated_jwt_secret(None).is_err());
        assert!(validated_jwt_secret(Some(String::new())).is_err());
    }

    #[test]
    fn the_placeholder_bootstrap_admin_password_is_rejected_but_unset_and_real_ones_pass() {
        assert!(checked_bootstrap_admin_password(Some("change-me-to-a-long-random-string")).is_err());
        assert!(checked_bootstrap_admin_password(Some("Change-Me")).is_err());
        assert!(checked_bootstrap_admin_password(None).is_ok());
        assert!(checked_bootstrap_admin_password(Some("")).is_ok());
        assert!(checked_bootstrap_admin_password(Some("admin123")).is_ok());
    }

    #[test]
    fn a_bind_all_public_url_is_rejected_on_a_real_domain() {
        for url in ["http://0.0.0.0:8080", "http://0.0.0.0", "https://0.0.0.0:8443/path", "http://[::]:8080"] {
            let error = validated_public_url(url.to_string(), "artiferris.example.com").unwrap_err();
            assert!(error.contains("PUBLIC_URL"), "{url}: {error}");
        }
    }

    #[test]
    fn a_bind_all_public_url_is_tolerated_on_a_local_dev_domain() {
        assert!(validated_public_url("http://0.0.0.0:8080".to_string(), "artiferris.localhost").is_ok());
    }

    #[test]
    fn a_real_public_url_passes_and_is_returned_unchanged() {
        for url in ["https://app.artiferris.example.com", "http://192.168.1.10:8080", "https://user@app.example.com:8443/x", "http://localhost:4200"] {
            assert_eq!(validated_public_url(url.to_string(), "artiferris.example.com").unwrap(), url);
        }
    }
}
