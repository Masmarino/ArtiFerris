use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::sso::{ExternalIdentity, LdapAuthPort, LdapConfig};
use ldap3::{LdapConnAsync, LdapConnSettings, ResultEntry, Scope, SearchEntry};
use std::time::Duration;

pub struct Ldap3AuthAdapter;

/// Covers connect, both binds and the search: a directory that stops answering must not hold a login request open.
const LDAP_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);
const LDAP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `ldaps://` is TLS from the first byte; `ldap://` is upgraded with StartTLS before any bind, so the service account and user passwords never cross the wire in cleartext.
fn connection_settings(server_url: &str) -> Result<LdapConnSettings, DomainError> {
    let settings = LdapConnSettings::new().set_conn_timeout(LDAP_CONNECT_TIMEOUT);
    match reqwest::Url::parse(server_url).map(|u| u.scheme().to_string()).as_deref() {
        Ok("ldaps") => Ok(settings),
        Ok("ldap") => Ok(settings.set_starttls(true)),
        _ => Err(DomainError::Infrastructure("ldap server_url must start with ldaps:// or ldap:// (which is upgraded with StartTLS)".to_string())),
    }
}

/// Escapes LDAP filter metacharacters per RFC 4515 before substituting into the template, so a crafted username can't alter the filter's structure.
fn build_filter(template: &str, username: &str) -> String {
    let escaped: String = username
        .chars()
        .map(|c| match c {
            '(' => "\\28".to_string(),
            ')' => "\\29".to_string(),
            '\\' => "\\5c".to_string(),
            '*' => "\\2a".to_string(),
            '\0' => "\\00".to_string(),
            other => other.to_string(),
        })
        .collect();
    template.replace("{username}", &escaped)
}

/// A search returns referrals and intermediate messages in the same list as the entries, and `SearchEntry::construct` panics on those.
fn entries_only(results: Vec<ResultEntry>) -> Vec<SearchEntry> {
    results.into_iter().filter(|result| !result.is_ref() && !result.is_intermediate()).map(SearchEntry::construct).collect()
}

/// Exactly one match required — zero or more than one both fail closed rather than guess.
fn extract_single_match(entries: Vec<SearchEntry>, email_attribute: &str) -> Result<(String, String), DomainError> {
    if entries.len() != 1 {
        return Err(DomainError::Infrastructure(format!("ldap search returned {} entries, expected exactly 1", entries.len())));
    }
    let entry = entries.into_iter().next().expect("length checked above");
    let email = entry
        .attrs
        .get(email_attribute)
        .and_then(|values| values.first())
        .ok_or_else(|| DomainError::Infrastructure(format!("ldap entry missing {email_attribute} attribute")))?
        .clone();
    Ok((entry.dn, email))
}

#[async_trait]
impl LdapAuthPort for Ldap3AuthAdapter {
    async fn authenticate(&self, config: &LdapConfig, username: &str, password: &str) -> Result<ExternalIdentity, DomainError> {
        // A non-empty DN with a zero-length password is an "unauthenticated bind" per RFC 4513 5.1.2 — some directories accept it regardless.
        if password.is_empty() {
            return Err(DomainError::Infrastructure("empty password rejected".to_string()));
        }

        let settings = connection_settings(&config.server_url)?;
        // Same SSRF guard as the other admin-configured remote hosts (npm/docker/OIDC).
        crate::ssrf_guard::ensure_public_host(&config.server_url).await?;

        exchange_within(LDAP_EXCHANGE_TIMEOUT, config, username, password, settings).await
    }
}

async fn exchange_within(deadline: Duration, config: &LdapConfig, username: &str, password: &str, settings: LdapConnSettings) -> Result<ExternalIdentity, DomainError> {
    tokio::time::timeout(deadline, exchange(config, username, password, settings))
        .await
        .map_err(|_| DomainError::Infrastructure("ldap server did not answer in time".to_string()))?
}

async fn exchange(config: &LdapConfig, username: &str, password: &str, settings: LdapConnSettings) -> Result<ExternalIdentity, DomainError> {
    let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &config.server_url).await.map_err(|e| DomainError::Infrastructure(format!("ldap connect failed: {e}")))?;
    ldap3::drive!(conn);

    ldap.simple_bind(&config.bind_dn, &config.bind_password)
        .await
        .map_err(|e| DomainError::Infrastructure(format!("ldap service bind failed: {e}")))?
        .success()
        .map_err(|e| DomainError::Infrastructure(format!("ldap service bind rejected: {e}")))?;

    let filter = build_filter(&config.user_search_filter, username);
    let (results, _) = ldap
        .search(&config.user_search_base, Scope::Subtree, &filter, vec![config.email_attribute.as_str()])
        .await
        .map_err(|e| DomainError::Infrastructure(format!("ldap search failed: {e}")))?
        .success()
        .map_err(|e| DomainError::Infrastructure(format!("ldap search rejected: {e}")))?;

    let (dn, email) = extract_single_match(entries_only(results), &config.email_attribute)?;

    // The actual credential check — the earlier service-account bind proves nothing about the submitted password.
    ldap.simple_bind(&dn, password)
        .await
        .map_err(|e| DomainError::Infrastructure(format!("ldap user bind failed: {e}")))?
        .success()
        .map_err(|_| DomainError::Infrastructure("ldap user bind rejected: invalid credentials".to_string()))?;

    let _ = ldap.unbind().await;
    Ok(ExternalIdentity { email, display_name: None })
}

#[cfg(test)]
mod tests {
    use artiferris_domain::sso::LdapConfig;
    use ldap3::asn1::{StructureTag, TagClass, PL};

    use super::*;

    #[tokio::test]
    async fn an_empty_password_is_rejected_without_contacting_the_directory() {
        let adapter = Ldap3AuthAdapter;
        let config = LdapConfig {
            server_url: "ldap://127.0.0.1:1".to_string(), // deliberately unroutable — if this test ever tries to connect, it will hang/fail slowly, proving the empty-password check didn't short-circuit
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        };

        let result = adapter.authenticate(&config, "florian", "").await;

        // Asserting on the message, not just is_err() — a connection failure here would also produce an Err, masking a removed check.
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("empty password"), "got {err:?}");
    }

    #[tokio::test]
    async fn a_server_url_pointing_at_a_private_address_is_rejected_before_connecting() {
        let adapter = Ldap3AuthAdapter;
        let config = LdapConfig {
            server_url: "ldap://127.0.0.1:1".to_string(),
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        };

        let err = adapter.authenticate(&config, "florian", "not-empty").await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn a_server_url_pointing_at_the_cloud_metadata_endpoint_is_rejected() {
        let adapter = Ldap3AuthAdapter;
        let config = LdapConfig {
            server_url: "ldap://169.254.169.254".to_string(),
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        };

        let err = adapter.authenticate(&config, "florian", "not-empty").await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    fn sample_config(server_url: &str) -> LdapConfig {
        LdapConfig {
            server_url: server_url.to_string(),
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        }
    }

    fn tag(class: TagClass, id: u64, payload: PL) -> StructureTag {
        StructureTag { class, id, payload }
    }

    fn octet_string(value: &str) -> StructureTag {
        tag(TagClass::Universal, 4, PL::P(value.as_bytes().to_vec()))
    }

    fn entry_result(dn: &str, email: &str) -> ResultEntry {
        let attribute = tag(TagClass::Universal, 16, PL::C(vec![octet_string("mail"), tag(TagClass::Universal, 17, PL::C(vec![octet_string(email)]))]));
        let attributes = tag(TagClass::Universal, 16, PL::C(vec![attribute]));
        ResultEntry::new(tag(TagClass::Application, 4, PL::C(vec![octet_string(dn), attributes])))
    }

    fn referral_result() -> ResultEntry {
        ResultEntry::new(tag(TagClass::Application, 19, PL::C(vec![octet_string("ldap://other.corp.example/dc=corp,dc=example")])))
    }

    fn intermediate_result() -> ResultEntry {
        ResultEntry::new(tag(TagClass::Application, 25, PL::C(vec![])))
    }

    #[test]
    fn referrals_and_intermediate_messages_are_skipped_instead_of_panicking() {
        let results = vec![referral_result(), entry_result("uid=florian,dc=corp,dc=example", "florian@corp.example"), intermediate_result(), referral_result()];

        let (dn, email) = extract_single_match(entries_only(results), "mail").unwrap();

        assert_eq!((dn.as_str(), email.as_str()), ("uid=florian,dc=corp,dc=example", "florian@corp.example"));
    }

    #[test]
    fn a_search_answered_only_with_referrals_finds_no_entry() {
        let err = extract_single_match(entries_only(vec![referral_result()]), "mail").unwrap_err();

        assert!(err.to_string().contains("returned 0 entries"), "got: {err}");
    }

    #[test]
    fn ldaps_is_tls_and_plain_ldap_is_upgraded_with_starttls() {
        assert!(!connection_settings("ldaps://ldap.corp.example").unwrap().starttls());
        assert!(connection_settings("ldap://ldap.corp.example:389").unwrap().starttls());
    }

    #[test]
    fn any_other_scheme_is_refused() {
        for url in ["http://ldap.corp.example", "ldapi:///var/run/ldapi", "ldap.corp.example", "", "ftp://x"] {
            let err = connection_settings(url).err().unwrap_or_else(|| panic!("{url} was accepted"));
            assert!(err.to_string().contains("ldaps://"), "{url}: {err}");
        }
    }

    #[tokio::test]
    async fn a_wrong_scheme_is_rejected_before_any_connection_attempt() {
        let err = Ldap3AuthAdapter.authenticate(&sample_config("http://8.8.8.8"), "florian", "not-empty").await.unwrap_err();

        assert!(err.to_string().contains("ldaps://"), "got: {err}");
    }

    #[tokio::test]
    async fn a_directory_that_accepts_the_connection_but_never_answers_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ldap://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let settings = connection_settings(&url).unwrap();

        let err = exchange_within(Duration::from_millis(300), &sample_config(&url), "florian", "not-empty", settings).await.unwrap_err();

        assert!(err.to_string().contains("did not answer in time"), "got: {err}");
    }

    #[test]
    fn substitutes_the_username_placeholder() {
        assert_eq!(build_filter("(uid={username})", "florian"), "(uid=florian)");
    }

    #[test]
    fn escapes_ldap_filter_metacharacters_in_the_submitted_username() {
        assert_eq!(build_filter("(uid={username})", "a)(uid=*"), "(uid=a\\29\\28uid=\\2a)");
    }

    #[test]
    fn leaves_a_filter_with_no_placeholder_unchanged() {
        assert_eq!(build_filter("(objectClass=person)", "florian"), "(objectClass=person)");
    }

    fn entry(dn: &str, attrs: Vec<(&str, Vec<&str>)>) -> SearchEntry {
        SearchEntry {
            dn: dn.to_string(),
            attrs: attrs.into_iter().map(|(k, vs)| (k.to_string(), vs.into_iter().map(str::to_string).collect())).collect(),
            bin_attrs: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn a_single_match_with_the_email_attribute_present_succeeds() {
        let entries = vec![entry("uid=florian,ou=people,dc=corp,dc=example", vec![("mail", vec!["florian@corp.example"])])];

        let result = extract_single_match(entries, "mail");

        assert_eq!(result.unwrap(), ("uid=florian,ou=people,dc=corp,dc=example".to_string(), "florian@corp.example".to_string()));
    }

    #[test]
    fn zero_matches_is_rejected() {
        let result = extract_single_match(vec![], "mail");

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("ldap search returned 0 entries, expected exactly 1"), "got {err:?}");
    }

    #[test]
    fn more_than_one_match_is_rejected() {
        let entries = vec![
            entry("uid=a,ou=people,dc=corp,dc=example", vec![("mail", vec!["a@corp.example"])]),
            entry("uid=b,ou=people,dc=corp,dc=example", vec![("mail", vec!["b@corp.example"])]),
        ];

        let result = extract_single_match(entries, "mail");

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("ldap search returned 2 entries, expected exactly 1"), "got {err:?}");
    }

    #[test]
    fn a_single_match_missing_the_email_attribute_is_rejected() {
        let entries = vec![entry("uid=florian,ou=people,dc=corp,dc=example", vec![("cn", vec!["Florian"])])];

        let result = extract_single_match(entries, "mail");

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("ldap entry missing mail attribute"), "got {err:?}");
    }

    #[test]
    fn a_single_match_with_an_empty_value_list_for_the_email_attribute_is_treated_as_missing() {
        // An empty Vec for the key takes the same "missing" path as the key being absent.
        let entries = vec![entry("uid=florian,ou=people,dc=corp,dc=example", vec![("mail", vec![])])];

        let result = extract_single_match(entries, "mail");

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("ldap entry missing mail attribute"), "got {err:?}");
    }
}
