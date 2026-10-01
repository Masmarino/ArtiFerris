use async_trait::async_trait;

use crate::error::DomainError;
use crate::npm_package::NpmPackageName;

#[async_trait]
pub trait RemoteNpmRegistryPort: Send + Sync {
    /// Raw JSON as received. With a username, `username` and `password` authenticate as HTTP Basic; with only a
    /// password, as a Bearer token (`_authToken`). `Ok(None)` for a genuine upstream 404.
    async fn fetch_metadata(
        &self,
        base_url: &str,
        package_name: &NpmPackageName,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<serde_json::Value>, DomainError>;

    /// `Ok(None)` for a genuine upstream 404 — not an error.
    async fn fetch_tarball(&self, base_url: &str, tarball_url: &str, username: Option<&str>, password: Option<&str>) -> Result<Option<Vec<u8>>, DomainError>;
}
