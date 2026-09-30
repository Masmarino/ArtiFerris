use async_trait::async_trait;

use crate::docker_registry::{ByteStream, Digest, DockerImageName};
use crate::error::DomainError;

pub struct RemoteBlob {
    /// What the remote declared, if it did. Not trusted: the body is verified as it is stored.
    pub content_length: Option<u64>,
    pub stream: ByteStream,
}

#[async_trait]
pub trait RemoteDockerRegistryPort: Send + Sync {
    /// `Ok(None)` means the remote genuinely returned 404 — expected, not an
    /// error. `Err` is reserved for real fetch failures. `username`/`password`
    /// authenticate to the upstream's token endpoint; `None` means anonymous.
    async fn fetch_manifest(
        &self,
        base_url: &str,
        image_name: &DockerImageName,
        reference: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<(Vec<u8>, String)>, DomainError>;

    /// Same `Ok(None)` == 404 convention as `fetch_manifest`. The body is streamed, never buffered whole.
    async fn fetch_blob(
        &self,
        base_url: &str,
        image_name: &DockerImageName,
        digest: &Digest,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<RemoteBlob>, DomainError>;
}
