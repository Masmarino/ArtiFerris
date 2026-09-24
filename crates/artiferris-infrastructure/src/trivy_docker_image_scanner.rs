use async_trait::async_trait;
use artiferris_domain::docker_scan::{DockerImageScannerPort, DockerVulnerability};
use artiferris_domain::error::DomainError;
use serde::Deserialize;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Wall-clock budget for a single `trivy` invocation; past this, the process is killed rather
/// than left to hang indefinitely (M-12).
const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Shells out to `trivy`, pointed at this deployment's own registry over loopback, authenticated with an internally-minted token (`--insecure` since it speaks plain HTTP internally).
pub struct TrivyDockerImageScanner {
    /// e.g. `127.0.0.1:8080` — not derived from the host external clients use.
    registry_host: String,
    /// Bounds how many `trivy` processes can run at once — unbounded concurrent pushes must not
    /// fork-bomb the host (M-12).
    concurrency: Arc<Semaphore>,
}

impl TrivyDockerImageScanner {
    pub fn new(registry_host: String) -> Self {
        Self { registry_host, concurrency: Arc::new(Semaphore::new(4)) }
    }

    /// Test-only constructor allowing the concurrency limit to be overridden, so a test can prove
    /// the semaphore actually bounds in-flight scans instead of just compiling (M-12).
    #[cfg(test)]
    fn with_concurrency_limit(registry_host: String, limit: usize) -> Self {
        Self { registry_host, concurrency: Arc::new(Semaphore::new(limit)) }
    }
}

/// Runs a prepared `Command`, killing it if it outlives `timeout`. Extracted so a test can drive
/// it directly against a trivial command instead of shelling out to a real `trivy` binary (M-12).
async fn run_scan_command(mut command: Command, timeout: std::time::Duration) -> Result<std::process::Output, DomainError> {
    command.kill_on_drop(true);
    tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| DomainError::Infrastructure("trivy scan timed out".to_string()))?
        .map_err(|e| DomainError::Infrastructure(format!("running trivy: {e}")))
}

#[async_trait]
impl DockerImageScannerPort for TrivyDockerImageScanner {
    async fn scan(
        &self,
        repository_name: &str,
        image_name: &str,
        reference: &str,
        platform: Option<&str>,
        mint_registry_token: &(dyn Fn() -> Result<String, DomainError> + Send + Sync),
    ) -> Result<Vec<DockerVulnerability>, DomainError> {
        // Bounds how many trivy processes can run at once — unbounded concurrent pushes must not
        // fork-bomb the host (M-12).
        let _permit = self.concurrency.acquire().await.map_err(|_| DomainError::Infrastructure("scanner shut down".to_string()))?;

        let image_ref = build_image_ref(&self.registry_host, repository_name, image_name, reference);

        // Only now: a token minted before the wait could be spent by the time a queued scan gets its turn.
        let registry_token = mint_registry_token()?;

        // Kept alive until the scan ends; dropping it deletes the token from disk.
        let credentials = write_registry_credentials(&self.registry_host, &registry_token)?;
        let command = build_scan_command(&image_ref, platform, credentials.path());

        let output = run_scan_command(command, SCAN_TIMEOUT).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::Infrastructure(format!("trivy scan of {image_ref} failed: {}", stderr.trim())));
        }
        parse_trivy_output(&output.stdout)
    }
}

/// A Docker config is matched per registry host, so the token only reaches ours. `TRIVY_REGISTRY_TOKEN`
/// would also be sent to mirror.gcr.io when Trivy downloads its vulnerability DB, which rejects it.
fn docker_config_json(registry_host: &str, registry_token: &str) -> String {
    serde_json::json!({ "auths": { registry_host: { "registrytoken": registry_token } } }).to_string()
}

/// A file, not a CLI arg (visible to any co-located process): 0700 dir, 0600 file, removed on drop.
fn write_registry_credentials(registry_host: &str, registry_token: &str) -> Result<tempfile::TempDir, DomainError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let io_error = |e: std::io::Error| DomainError::Infrastructure(format!("writing trivy registry credentials: {e}"));
    let dir = tempfile::Builder::new().prefix("trivy-docker-config-").tempdir().map_err(io_error)?;
    std::fs::set_permissions(dir.path(), std::os::unix::fs::PermissionsExt::from_mode(0o700)).map_err(io_error)?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dir.path().join("config.json")).map_err(io_error)?;
    file.write_all(docker_config_json(registry_host, registry_token).as_bytes()).map_err(io_error)?;
    Ok(dir)
}

/// The only parent variables Trivy gets: enough to find its cache and reach its DB mirror, never the server's own secrets.
const TRIVY_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "TRIVY_CACHE_DIR",
    "TRIVY_DB_REPOSITORY",
    "TRIVY_JAVA_DB_REPOSITORY",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

fn build_scan_command(image_ref: &str, platform: Option<&str>, docker_config_dir: &std::path::Path) -> Command {
    scan_command("trivy".as_ref(), image_ref, platform, docker_config_dir, std::env::vars())
}

/// Trivy parses attacker-supplied image layers, so it starts from an empty environment.
fn scan_command(program: &std::ffi::OsStr, image_ref: &str, platform: Option<&str>, docker_config_dir: &std::path::Path, parent_env: impl Iterator<Item = (String, String)>) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    command.envs(parent_env.filter(|(name, _)| TRIVY_ENV_ALLOWLIST.contains(&name.as_str())));
    command.arg("image").arg("--format").arg("json").arg("--quiet").arg("--insecure");
    command.env("DOCKER_CONFIG", docker_config_dir);
    if let Some(platform) = platform {
        command.arg("--platform").arg(platform);
    }
    command.arg(image_ref);
    command
}

/// A tag uses `:`, a digest uses `@` — conflating the two produces a reference `trivy`/`go-containerregistry` reject outright.
fn build_image_ref(registry_host: &str, repository_name: &str, image_name: &str, reference: &str) -> String {
    let separator = if reference.starts_with("sha256:") { "@" } else { ":" };
    format!("{registry_host}/{repository_name}/{image_name}{separator}{reference}")
}

#[derive(Debug, Deserialize)]
struct TrivyReport {
    #[serde(rename = "Results", default)]
    results: Vec<TrivyResult>,
}

#[derive(Debug, Deserialize)]
struct TrivyResult {
    #[serde(rename = "Vulnerabilities", default)]
    vulnerabilities: Vec<TrivyVulnerability>,
}

#[derive(Debug, Deserialize)]
struct TrivyVulnerability {
    #[serde(rename = "VulnerabilityID")]
    vulnerability_id: String,
    #[serde(rename = "PkgName")]
    pkg_name: String,
    #[serde(rename = "InstalledVersion")]
    installed_version: String,
    #[serde(rename = "FixedVersion", default)]
    fixed_version: Option<String>,
    #[serde(rename = "Severity", default)]
    severity: Option<String>,
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "PrimaryURL", default)]
    primary_url: Option<String>,
}

fn parse_trivy_output(bytes: &[u8]) -> Result<Vec<DockerVulnerability>, DomainError> {
    let report: TrivyReport = serde_json::from_slice(bytes).map_err(|e| DomainError::Infrastructure(format!("parsing trivy output: {e}")))?;
    Ok(report
        .results
        .into_iter()
        .flat_map(|r| r.vulnerabilities)
        .map(|v| DockerVulnerability {
            id: v.vulnerability_id,
            package_name: v.pkg_name,
            installed_version: v.installed_version,
            fixed_version: v.fixed_version,
            severity: v.severity.unwrap_or_else(|| "UNKNOWN".to_string()),
            title: v.title,
            primary_url: v.primary_url,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_docker_config_scopes_the_token_to_the_registry_host_only() {
        let config: serde_json::Value = serde_json::from_str(&docker_config_json("127.0.0.1:8080", "the-token")).unwrap();
        let auths = config["auths"].as_object().unwrap();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths["127.0.0.1:8080"]["registrytoken"], "the-token");
    }

    #[test]
    fn a_token_with_json_special_characters_is_escaped_not_injected() {
        let config: serde_json::Value = serde_json::from_str(&docker_config_json("127.0.0.1:8080", r#"a"b\c"#)).unwrap();
        assert_eq!(config["auths"]["127.0.0.1:8080"]["registrytoken"], r#"a"b\c"#);
    }

    #[test]
    fn the_credentials_are_written_owner_only_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let dir = write_registry_credentials("127.0.0.1:8080", "the-token").unwrap();
        let file = dir.path().join("config.json");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(std::fs::read_to_string(&file).unwrap().contains("the-token"));

        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn the_scan_command_gets_a_scoped_docker_config_and_never_the_global_registry_token() {
        let command = build_scan_command("127.0.0.1:8080/r/i:t", None, std::path::Path::new("/tmp/cfg"));
        let envs: Vec<_> = command.as_std().get_envs().collect();
        assert!(envs.iter().any(|(k, v)| *k == "DOCKER_CONFIG" && *v == Some(std::ffi::OsStr::new("/tmp/cfg"))));
        assert!(envs.iter().all(|(k, _)| *k != "TRIVY_REGISTRY_TOKEN"));
    }

    #[tokio::test]
    async fn trivy_never_sees_the_servers_secrets() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_trivy = dir.path().join("trivy");
        std::fs::write(&fake_trivy, "#!/bin/sh\nenv\n").unwrap();
        std::fs::set_permissions(&fake_trivy, std::fs::Permissions::from_mode(0o755)).unwrap();

        // A real variable in this process, so the check fails if the child ever inherits the parent's environment.
        unsafe { std::env::set_var("ARTIFERRIS_TEST_CANARY_SECRET", "canary-secret-value") };
        let command = scan_command(fake_trivy.as_os_str(), "127.0.0.1:8080/r/i:t", None, std::path::Path::new("/tmp/cfg"), std::env::vars());

        let output = run_scan_command(command, std::time::Duration::from_secs(10)).await.unwrap();
        let seen = String::from_utf8(output.stdout).unwrap();

        assert!(seen.contains("DOCKER_CONFIG=/tmp/cfg"), "got: {seen}");
        assert!(seen.contains("PATH="), "got: {seen}");
        assert!(!seen.contains("canary-secret-value") && !seen.contains("ARTIFERRIS_TEST_CANARY_SECRET"), "trivy inherited the parent environment: {seen}");
    }

    #[test]
    fn only_allow_listed_variables_are_forwarded_to_trivy() {
        let parent_env = [("PATH", "/usr/bin"), ("HTTPS_PROXY", "http://proxy:3128"), ("JWT_SECRET", "x"), ("SECRETS_ENCRYPTION_KEY", "y"), ("DATABASE_URL", "z")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()));
        let command = scan_command("trivy".as_ref(), "img", None, std::path::Path::new("/tmp/cfg"), parent_env);
        let names: Vec<String> = command.as_std().get_envs().filter_map(|(k, v)| v.map(|_| k.to_string_lossy().into_owned())).collect();

        assert!(names.contains(&"PATH".to_string()) && names.contains(&"HTTPS_PROXY".to_string()) && names.contains(&"DOCKER_CONFIG".to_string()));
        for secret in ["JWT_SECRET", "SECRETS_ENCRYPTION_KEY", "DATABASE_URL"] {
            assert!(!names.contains(&secret.to_string()), "{secret} was forwarded");
        }
    }

    #[test]
    fn builds_a_tag_reference_with_a_colon() {
        assert_eq!(build_image_ref("127.0.0.1:8080", "myrepo", "myimage", "latest"), "127.0.0.1:8080/myrepo/myimage:latest");
    }

    #[test]
    fn builds_a_digest_reference_with_an_at_sign() {
        let digest = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(build_image_ref("127.0.0.1:8080", "myrepo", "myimage", digest), format!("127.0.0.1:8080/myrepo/myimage@{digest}"));
    }

    #[test]
    fn parses_real_trivy_json_output_into_flat_vulnerabilities() {
        let fixture = serde_json::json!({
            "Results": [
                {
                    "Target": "myimage (alpine 3.15.4)",
                    "Class": "os-pkgs",
                    "Vulnerabilities": [
                        {
                            "VulnerabilityID": "CVE-2022-4450",
                            "PkgName": "libcrypto1.1",
                            "InstalledVersion": "1.1.1n-r0",
                            "FixedVersion": "1.1.1t-r0",
                            "Severity": "HIGH",
                            "Title": "openssl: double free after calling PEM_read_bio_ex",
                            "PrimaryURL": "https://avd.aquasec.com/nvd/cve-2022-4450"
                        }
                    ]
                },
                {
                    "Target": "myimage (clean layer)",
                    "Class": "os-pkgs"
                }
            ]
        });
        let vulnerabilities = parse_trivy_output(fixture.to_string().as_bytes()).unwrap();

        assert_eq!(vulnerabilities.len(), 1);
        assert_eq!(vulnerabilities[0].id, "CVE-2022-4450");
        assert_eq!(vulnerabilities[0].package_name, "libcrypto1.1");
        assert_eq!(vulnerabilities[0].fixed_version, Some("1.1.1t-r0".to_string()));
        assert_eq!(vulnerabilities[0].severity, "HIGH");
    }

    #[test]
    fn a_report_with_no_results_at_all_parses_to_an_empty_list() {
        let vulnerabilities = parse_trivy_output(b"{}").unwrap();
        assert!(vulnerabilities.is_empty());
    }

    #[test]
    fn a_vulnerability_missing_a_severity_defaults_to_unknown() {
        let fixture = serde_json::json!({
            "Results": [{
                "Vulnerabilities": [{
                    "VulnerabilityID": "CVE-0000-0000",
                    "PkgName": "somepkg",
                    "InstalledVersion": "1.0.0"
                }]
            }]
        });
        let vulnerabilities = parse_trivy_output(fixture.to_string().as_bytes()).unwrap();
        assert_eq!(vulnerabilities[0].severity, "UNKNOWN");
        assert_eq!(vulnerabilities[0].fixed_version, None);
    }

    #[tokio::test]
    async fn a_scan_that_runs_too_long_is_killed_and_times_out() {
        // Requires the scanner's process-invocation to be injectable for a test — see Step 3's
        // `run_scan_command` extraction. Construct a scanner (or call the extracted helper directly)
        // with a command that sleeps past the configured timeout (e.g. `Command::new("sleep").arg("2")`)
        // and a short test-only timeout override, and assert the call returns an error within a bounded
        // wall-clock window rather than the full sleep duration.
        let mut command = tokio::process::Command::new("sleep");
        command.arg("2");
        let started = std::time::Instant::now();
        let result = run_scan_command(command, std::time::Duration::from_millis(200)).await;
        assert!(result.is_err(), "a command that outlives the timeout must error, not succeed");
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "must not wait for the full subprocess duration");
    }

    #[tokio::test]
    async fn the_concurrency_semaphore_caps_in_flight_scans_at_the_configured_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 6 tasks contend for 2 permits. If the semaphore genuinely bounds concurrency, the
        // observed in-flight count must both reach 2 (proving tasks really do run concurrently,
        // ruling out a test that's accidentally sequential) and never exceed 2 (proving the cap
        // holds) — a test that just runs scans one after another wouldn't exercise either half.
        const LIMIT: usize = 2;
        let scanner = TrivyDockerImageScanner::with_concurrency_limit("127.0.0.1:0".to_string(), LIMIT);
        let concurrency = scanner.concurrency.clone();

        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_observed = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..6)
            .map(|_| {
                let concurrency = concurrency.clone();
                let in_flight = in_flight.clone();
                let max_observed = max_observed.clone();
                tokio::spawn(async move {
                    // Mirrors what `scan` does: hold a permit for the duration of the subprocess.
                    let _permit = concurrency.acquire().await.unwrap();
                    let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_observed.fetch_max(current, Ordering::SeqCst);

                    let mut command = tokio::process::Command::new("sleep");
                    command.arg("0.2");
                    run_scan_command(command, std::time::Duration::from_secs(5)).await.unwrap();

                    in_flight.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(
            max_observed.load(Ordering::SeqCst),
            LIMIT,
            "in-flight scan count must reach but never exceed the configured concurrency limit"
        );
    }
}
