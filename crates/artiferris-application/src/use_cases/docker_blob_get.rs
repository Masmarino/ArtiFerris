use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use artiferris_domain::docker_registry::{ByteStream, Digest, DockerBlobStorePort, DockerImageName, DockerManifestRepositoryPort, MAX_BLOB_BYTES};
use artiferris_domain::docker_remote::RemoteDockerRegistryPort;
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use futures::StreamExt;
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::keyed_locks::KeyedLocks;
use crate::use_cases::group_resolve::resolve_in_group;

/// Longest a proxy cache fill may take, however slowly the upstream trickles.
const PROXY_FILL_DEADLINE: Duration = Duration::from_secs(30 * 60);

/// How far the fill may run ahead of the client that is reading it.
const FILL_CHANNEL_CHUNKS: usize = 4;

/// A client that takes no chunk for this long is let go, so it can't hold the fill (and everyone waiting for it) back.
const CLIENT_STALL_LIMIT: Duration = Duration::from_secs(30);

pub struct GetBlobUseCase {
    blobs: Arc<dyn DockerBlobStorePort>,
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    remote: Arc<dyn RemoteDockerRegistryPort>,
    max_blob_bytes: u64,
    /// One cache fill per (repository, digest) at a time; the others wait for it and read the result.
    fills: KeyedLocks<(Uuid, String)>,
    client_stall_limit: Duration,
    /// Serializes the quota check of a repository's fills.
    budgets: KeyedLocks<Uuid>,
    /// Bytes promised to fills in progress, per repository.
    promised: Arc<Mutex<HashMap<Uuid, u64>>>,
}

/// Bytes of a repository's quota held by a fill in progress, given back on drop.
struct Reservation {
    promised: Arc<Mutex<HashMap<Uuid, u64>>>,
    repository_id: Uuid,
    bytes: u64,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut promised = self.promised.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(total) = promised.get_mut(&self.repository_id) {
            *total = total.saturating_sub(self.bytes);
            if *total == 0 {
                promised.remove(&self.repository_id);
            }
        }
    }
}

impl GetBlobUseCase {
    pub fn new(blobs: Arc<dyn DockerBlobStorePort>, manifests: Arc<dyn DockerManifestRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>, remote: Arc<dyn RemoteDockerRegistryPort>) -> Self {
        Self {
            blobs,
            manifests,
            repositories,
            remote,
            max_blob_bytes: MAX_BLOB_BYTES,
            fills: KeyedLocks::new(),
            client_stall_limit: CLIENT_STALL_LIMIT,
            budgets: KeyedLocks::new(),
            promised: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_client_stall_limit(mut self, limit: Duration) -> Self {
        self.client_stall_limit = limit;
        self
    }

    pub fn with_max_blob_bytes(mut self, max_blob_bytes: u64) -> Self {
        self.max_blob_bytes = max_blob_bytes;
        self
    }

    /// The blob's bytes, chunked. `authorize_member` is the caller's read policy, consulted for every group member the
    /// traversal would descend into — the top-level repository's own access is the caller's responsibility, checked once
    /// before this is ever called (C-1). A proxy serves what it has cached and otherwise fills its cache from the remote,
    /// verified against `digest` on the way to disk.
    pub fn execute_stream<'a, FAuthorize, FutAuthorize>(
        &'a self,
        repository_id: Uuid,
        image_name: &'a DockerImageName,
        digest: &'a Digest,
        authorize_member: FAuthorize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<ByteStream>, ApplicationError>> + Send + 'a>>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
        FutAuthorize: std::future::Future<Output = bool> + Send + 'a,
    {
        Box::pin(async move {
            resolve_in_group(
                &self.repositories,
                repository_id,
                HashSet::new(),
                // Blob storage is globally deduped, but a read must stay scoped to blobs this repository actually has, or any caller could read any repository's content by digest alone.
                move |repository_id| async move {
                    if self.is_reachable(repository_id, digest).await? {
                        Ok(Some(self.blobs.read_stream(digest).await?))
                    } else {
                        Ok(None)
                    }
                },
                move |repository_id, repo| self.proxy_blob_stream(repository_id, repo, image_name, digest),
                || ApplicationError::DockerManifestNotFound,
                authorize_member,
            )
            .await
        })
    }

    /// Hot path for `docker push`'s per-layer `HEAD` check: answered from the local size, no blob bytes read.
    pub fn execute_exists<'a, FAuthorize, FutAuthorize>(
        &'a self,
        repository_id: Uuid,
        image_name: &'a DockerImageName,
        digest: &'a Digest,
        authorize_member: FAuthorize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<u64>, ApplicationError>> + Send + 'a>>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
        FutAuthorize: std::future::Future<Output = bool> + Send + 'a,
    {
        Box::pin(async move {
            // The size lookup is global (deduped by digest across every repository), so it must never be trusted on its
            // own — that would turn HEAD into an oracle for "does this blob exist anywhere on the server" (C-2).
            if let Some(size_bytes) = self.local_size(repository_id, digest).await? {
                return Ok(Some(size_bytes));
            }
            resolve_in_group(
                &self.repositories,
                repository_id,
                HashSet::new(),
                move |repository_id| self.local_size(repository_id, digest),
                move |repository_id, repo| self.proxy_blob_size(repository_id, repo, image_name, digest),
                || ApplicationError::DockerManifestNotFound,
                authorize_member,
            )
            .await
        })
    }

    async fn is_reachable(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, ApplicationError> {
        Ok(self.manifests.blob_is_reachable(repository_id, digest).await? || self.blobs.is_uploaded_to_repository(repository_id, digest).await?)
    }

    /// The size of a blob this repository can reach and the store actually holds.
    async fn local_size(&self, repository_id: Uuid, digest: &Digest) -> Result<Option<u64>, ApplicationError> {
        if !self.is_reachable(repository_id, digest).await? {
            return Ok(None);
        }
        Ok(self.blobs.size_if_exists(digest).await?)
    }

    async fn proxy_blob_stream(&self, repository_id: Uuid, repo: PackageRepositorySummary, image_name: &DockerImageName, digest: &Digest) -> Result<Option<ByteStream>, ApplicationError> {
        if self.local_size(repository_id, digest).await?.is_some() {
            return Ok(Some(self.blobs.read_stream(digest).await?));
        }
        let fill = self.fills.lock((repository_id, digest.as_str().to_string())).await;
        // A request that held the lock before this one may have filled the cache.
        if self.local_size(repository_id, digest).await?.is_some() {
            drop(fill);
            return Ok(Some(self.blobs.read_stream(digest).await?));
        }
        self.start_fill(fill, repository_id, &repo, image_name, digest).await
    }

    async fn proxy_blob_size(&self, repository_id: Uuid, repo: PackageRepositorySummary, image_name: &DockerImageName, digest: &Digest) -> Result<Option<u64>, ApplicationError> {
        if let Some(size_bytes) = self.local_size(repository_id, digest).await? {
            return Ok(Some(size_bytes));
        }
        // The remote's own answer is enough for a HEAD: the body is dropped unread.
        let remote_url = Self::remote_url(&repo)?;
        let Some(remote) = self.remote.fetch_blob(remote_url, image_name, digest, repo.remote_username.as_deref(), repo.remote_password.as_deref()).await? else {
            return Ok(None);
        };
        if let Some(content_length) = remote.content_length {
            return Ok(Some(content_length));
        }
        drop(remote);
        // No declared length: fills the cache and reads the size from there, once the fill is done.
        if self.proxy_blob_stream(repository_id, repo, image_name, digest).await?.is_none() {
            return Ok(None);
        }
        drop(self.fills.lock((repository_id, digest.as_str().to_string())).await);
        self.local_size(repository_id, digest).await
    }

    fn remote_url(repo: &PackageRepositorySummary) -> Result<&str, ApplicationError> {
        repo.remote_url.as_deref().ok_or_else(|| ApplicationError::InvalidDockerPayload("proxy repository has no remote_url configured".into()))
    }

    /// Fills the cache in a task of its own, so a client going away doesn't cancel it, and hands back the bytes as they arrive.
    /// The digest is only checked once the blob is all in, so a bad blob ends the stream with an error. `Ok(None)` if the remote
    /// has no such blob.
    async fn start_fill(
        &self,
        fill: OwnedMutexGuard<()>,
        repository_id: Uuid,
        repo: &PackageRepositorySummary,
        image_name: &DockerImageName,
        digest: &Digest,
    ) -> Result<Option<ByteStream>, ApplicationError> {
        let remote_url = Self::remote_url(repo)?;
        // `None` means the remote genuinely 404'd — a real "not found", not an infra failure.
        let Some(remote) = self.remote.fetch_blob(remote_url, image_name, digest, repo.remote_username.as_deref(), repo.remote_password.as_deref()).await? else {
            return Ok(None);
        };
        if remote.content_length.is_some_and(|declared| declared > self.max_blob_bytes) {
            return Err(ApplicationError::DockerUploadTooLarge);
        }
        let (limit, reservation) = self.reserve_room(repository_id, repo, remote.content_length).await?;

        let (sender, receiver) = tokio::sync::mpsc::channel::<bytes::Bytes>(FILL_CHANNEL_CHUNKS);
        // `failure` is what ends the client's stream when that isn't the end of a good blob.
        let client = Arc::new(tokio::sync::Mutex::new(Some(sender)));
        let failure: Arc<Mutex<Option<DomainError>>> = Arc::default();
        let blobs = self.blobs.clone();
        let digest = digest.clone();
        let (task_client, task_failure, stall_limit) = (client.clone(), failure.clone(), self.client_stall_limit);
        tokio::spawn(async move {
            let _fill = fill;
            let _reservation = reservation;
            let (tee_client, tee_failure) = (task_client.clone(), task_failure.clone());
            let forwarded = remote.stream.then(move |item| {
                let (client, failure) = (tee_client.clone(), tee_failure.clone());
                async move {
                    if let Ok(chunk) = &item {
                        let mut client = client.lock().await;
                        if let Some(sender) = client.as_ref() {
                            match tokio::time::timeout(stall_limit, sender.send(chunk.clone())).await {
                                Ok(Ok(())) => {}
                                // Gone: the fill carries on alone.
                                Ok(Err(_)) => *client = None,
                                Err(_) => {
                                    *failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(DomainError::Infrastructure("the client stopped reading the blob".into()));
                                    *client = None;
                                }
                            }
                        }
                    }
                    item
                }
            });
            let stored = match tokio::time::timeout(PROXY_FILL_DEADLINE, blobs.write_stream(&digest, Box::pin(forwarded), limit)).await {
                Ok(Ok(_)) => blobs.link_to_repository(repository_id, &digest).await,
                Ok(Err(e)) => Err(e),
                Err(_) => Err(DomainError::Infrastructure(format!("fetching {} took longer than {PROXY_FILL_DEADLINE:?}", digest.as_str()))),
            };
            if let Err(e) = stored {
                tracing::warn!(%repository_id, digest = digest.as_str(), error = %e, "filling the proxy cache failed");
                task_failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_or_insert(e);
            }
            // After `failure` is set, so the client can't mistake the end of a bad blob for the end of a good one.
            task_client.lock().await.take();
        });
        Ok(Some(Box::pin(futures::stream::unfold((receiver, failure), |(mut receiver, failure)| async move {
            match receiver.recv().await {
                Some(chunk) => Some((Ok(chunk), (receiver, failure))),
                None => {
                    let failed = failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
                    failed.map(|e| (Err(e), (receiver, failure)))
                }
            }
        }))))
    }

    /// The most a fill may store, and the quota it holds until dropped: what is left after what the repository holds and
    /// what fills in progress have promised. Without a quota, only the size cap applies.
    async fn reserve_room(&self, repository_id: Uuid, repo: &PackageRepositorySummary, declared: Option<u64>) -> Result<(u64, Option<Reservation>), ApplicationError> {
        let Some(quota) = repo.quota_bytes else {
            return Ok((self.max_blob_bytes, None));
        };
        let _turn = self.budgets.lock(repository_id).await;
        // What is promised is read before what is stored: a fill that finishes in between is then counted twice, never missed.
        let promised = self.promised.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&repository_id).copied().unwrap_or(0);
        let used = self.blobs.used_bytes_for_repository(repository_id).await?;
        let room = (quota.max(0) as u64).saturating_sub(used.saturating_add(promised));
        let limit = room.min(self.max_blob_bytes);
        if declared.is_some_and(|declared| declared > limit) {
            return Err(ApplicationError::StorageQuotaExceeded);
        }
        let bytes = declared.unwrap_or(limit);
        *self.promised.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).entry(repository_id).or_insert(0) += bytes;
        Ok((limit, Some(Reservation { promised: self.promised.clone(), repository_id, bytes })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeDockerManifestRepository, FakeRemoteDockerRegistry, FakeRepositories};
    use artiferris_domain::docker_registry::{DockerImageName, DockerManifest, DockerMediaType};

    fn hosted_repo(id: Uuid) -> artiferris_domain::package_repository::PackageRepositorySummary {
        artiferris_domain::package_repository::PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(), name: format!("repo-{id}"), format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        }
    }

    async fn read_all(stream: Option<ByteStream>) -> Option<Vec<u8>> {
        use futures::StreamExt;
        let mut stream = stream?;
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        Some(collected)
    }

    fn proxy_repo(id: Uuid, quota_bytes: Option<i64>) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id, organization_id: Uuid::new_v4(), name: "proxy-repo".to_string(), format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Proxy, remote_url: Some("https://registry-1.docker.io".to_string()), remote_username: None, remote_password: None,
            quota_bytes, retention_keep_last_n: None, is_public: false, group_members: vec![],
        }
    }

    fn allow_all() -> impl Fn(&PackageRepositorySummary) -> std::future::Ready<bool> + Clone + Send {
        |_repo: &PackageRepositorySummary| std::future::ready(true)
    }

    /// Links `digest` to a manifest actually stored in `repository_id`, the way a real push would — the fixture other tests need to make a blob reachable.
    async fn link_blob_to_repository(manifests: &FakeDockerManifestRepository, repository_id: Uuid, digest: &Digest) {
        let manifest = DockerManifest {
            id: Uuid::new_v4(), package_repository_id: repository_id, image_name: DockerImageName::parse("anything").unwrap(),
            digest: Digest::of(b"manifest-bytes"), media_type: DockerMediaType::DockerV2Manifest, body: b"{}".to_vec(), created_at: chrono::Utc::now(),
        };
        manifests.insert_manifest(&manifest, &[digest.clone()]).await.unwrap();
    }

    #[tokio::test]
    async fn a_blob_reachable_from_the_requested_repository_is_served() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"layer-bytes");
        blobs.write(&digest, b"layer-bytes").await.unwrap();
        let repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, repository_id, &digest).await;
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(hosted_repo(repository_id));

        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));
        let stream = use_case.execute_stream(repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert_eq!(read_all(stream).await, Some(b"layer-bytes".to_vec()));
    }

    /// Blob storage is globally deduplicated by digest, but a repository the caller has no manifest in must not be able to serve content that only ever lived elsewhere.
    #[tokio::test]
    async fn a_blob_only_reachable_from_a_different_repository_is_not_served() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"private-layer-bytes");
        blobs.write(&digest, b"private-layer-bytes").await.unwrap();
        let other_repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, other_repository_id, &digest).await;

        let repositories = Arc::new(FakeRepositories::new());
        let requested_repository_id = Uuid::new_v4();
        repositories.insert(hosted_repo(requested_repository_id));
        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));

        let result = use_case.execute_stream(requested_repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert!(result.is_none(), "a blob linked only to another repository must not be readable through this one");
    }

    #[tokio::test]
    async fn execute_stream_yields_the_bytes_of_a_reachable_blob() {
        use futures::StreamExt;

        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"streamed-layer-bytes");
        blobs.write(&digest, b"streamed-layer-bytes").await.unwrap();
        let repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, repository_id, &digest).await;
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(hosted_repo(repository_id));

        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));
        let mut stream = use_case.execute_stream(repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap().unwrap();
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, b"streamed-layer-bytes");
    }

    #[tokio::test]
    async fn execute_stream_of_an_unreachable_blob_returns_none() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"private-layer-bytes");
        blobs.write(&digest, b"private-layer-bytes").await.unwrap();
        let other_repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, other_repository_id, &digest).await;

        let repositories = Arc::new(FakeRepositories::new());
        let requested_repository_id = Uuid::new_v4();
        repositories.insert(hosted_repo(requested_repository_id));
        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));

        let result = use_case.execute_stream(requested_repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn a_missing_blob_against_a_hosted_repository_returns_none() {
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(hosted_repo(repository_id));
        let use_case = GetBlobUseCase::new(
            Arc::new(FakeDockerBlobStore::new()),
            Arc::new(FakeDockerManifestRepository::new()),
            repositories,
            Arc::new(FakeRemoteDockerRegistry::new()),
        );
        let result = use_case.execute_stream(repository_id, &DockerImageName::parse("anything").unwrap(), &Digest::of(b"never-uploaded"), |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn a_missing_blob_is_fetched_from_the_proxys_remote_registry() {
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repository_id,
            organization_id: Uuid::new_v4(), name: "proxy-repo".to_string(), format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Proxy, remote_url: Some("https://registry-1.docker.io".to_string()), remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        });
        let use_case = GetBlobUseCase::new(Arc::new(FakeDockerBlobStore::new()), Arc::new(FakeDockerManifestRepository::new()), repositories, Arc::new(FakeRemoteDockerRegistry::new()));

        // FakeRemoteDockerRegistry::fetch_blob always answers with these exact bytes.
        let digest = Digest::of(b"fake-blob-bytes");
        let stream = use_case.execute_stream(repository_id, &DockerImageName::parse("library/alpine").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert_eq!(read_all(stream).await, Some(b"fake-blob-bytes".to_vec()));
    }

    #[tokio::test]
    async fn a_cyclic_group_configuration_resolves_without_hanging() {
        let repo_a_id = Uuid::new_v4();
        let repo_b_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repo_a_id,
            organization_id: Uuid::new_v4(), name: "group-a".to_string(), format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Group, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![repo_b_id],
        });
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repo_b_id,
            organization_id: Uuid::new_v4(), name: "group-b".to_string(), format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Group, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![repo_a_id],
        });

        let use_case = GetBlobUseCase::new(Arc::new(FakeDockerBlobStore::new()), Arc::new(FakeDockerManifestRepository::new()), repositories, Arc::new(FakeRemoteDockerRegistry::new()));
        let name = DockerImageName::parse("myimage").unwrap();

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), use_case.execute_stream(repo_a_id, &name, &Digest::of(b"never-uploaded"), |_repo: &PackageRepositorySummary| async { true }))
            .await
            .expect("execute() must not hang on a cyclic group configuration");
        assert!(result.unwrap().is_none());
    }

    /// `execute_exists` must not answer from the global, dedup-by-digest store alone — that turns
    /// HEAD into an oracle for "does this blob exist anywhere on the server", leaking the existence
    /// of content in repositories the caller can't read (C-2).
    #[tokio::test]
    async fn exists_of_a_blob_only_reachable_from_a_different_repository_returns_none() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"private-layer-bytes");
        blobs.write(&digest, b"private-layer-bytes").await.unwrap();
        let other_repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, other_repository_id, &digest).await;

        let repositories = Arc::new(FakeRepositories::new());
        let requested_repository_id = Uuid::new_v4();
        repositories.insert(hosted_repo(requested_repository_id));
        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));

        let result = use_case.execute_exists(requested_repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert!(result.is_none(), "a blob linked only to another repository must not be reported as existing through this one");
    }

    #[tokio::test]
    async fn exists_of_a_blob_reachable_from_the_requested_repository_returns_its_size() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let digest = Digest::of(b"layer-bytes");
        blobs.write(&digest, b"layer-bytes").await.unwrap();
        let repository_id = Uuid::new_v4();
        link_blob_to_repository(&manifests, repository_id, &digest).await;
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(hosted_repo(repository_id));

        let use_case = GetBlobUseCase::new(blobs, manifests, repositories, Arc::new(FakeRemoteDockerRegistry::new()));
        let result = use_case.execute_exists(repository_id, &DockerImageName::parse("anything").unwrap(), &digest, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert_eq!(result, Some(b"layer-bytes".len() as u64));
    }

    fn proxy_use_case(repository: PackageRepositorySummary, remote: Arc<FakeRemoteDockerRegistry>) -> (Arc<GetBlobUseCase>, Arc<FakeDockerBlobStore>) {
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(repository);
        let blobs = Arc::new(FakeDockerBlobStore::new());
        (Arc::new(GetBlobUseCase::new(blobs.clone(), Arc::new(FakeDockerManifestRepository::new()), repositories, remote)), blobs)
    }

    #[tokio::test]
    async fn a_proxied_blob_is_fetched_once_and_then_served_from_the_cache() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"a layer of the proxied image");
        let (use_case, blobs) = proxy_use_case(proxy_repo(repository_id, None), remote.clone());
        let name = DockerImageName::parse("library/alpine").unwrap();

        for _ in 0..3 {
            let stream = use_case.execute_stream(repository_id, &name, &digest, allow_all()).await.unwrap();
            assert_eq!(read_all(stream).await, Some(b"a layer of the proxied image".to_vec()));
        }

        assert_eq!(remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst), 1, "the upstream is asked once, not on every pull");
        assert!(blobs.is_uploaded_to_repository(repository_id, &digest).await.unwrap());
    }

    #[tokio::test]
    async fn a_proxied_blob_that_does_not_hash_to_its_digest_is_refused_and_not_cached() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let (use_case, blobs) = proxy_use_case(proxy_repo(repository_id, None), remote);
        let name = DockerImageName::parse("library/alpine").unwrap();
        let asked_for = Digest::of(b"the layer the client asked for");

        // The remote answers with other bytes ("fake-blob-bytes"); the mismatch is only known once they are all in.
        let stream = use_case.execute_stream(repository_id, &name, &asked_for, allow_all()).await.unwrap().unwrap();
        let items: Vec<_> = stream.collect().await;

        assert!(matches!(items.last(), Some(Err(DomainError::DigestMismatch { .. }))), "the stream must end in an error, got {items:?}");
        assert!(!blobs.exists(&asked_for).await.unwrap());
        assert!(!blobs.exists(&Digest::of(b"fake-blob-bytes")).await.unwrap(), "the bytes that did arrive are not kept under any digest");
        assert!(!blobs.is_uploaded_to_repository(repository_id, &asked_for).await.unwrap());
    }

    #[tokio::test]
    async fn a_proxied_blob_larger_than_the_limit_is_refused_and_not_cached() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"more bytes than the limit allows");
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(proxy_repo(repository_id, None));
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let use_case = GetBlobUseCase::new(blobs.clone(), Arc::new(FakeDockerManifestRepository::new()), repositories, remote).with_max_blob_bytes(8);

        let result = use_case.execute_stream(repository_id, &DockerImageName::parse("library/alpine").unwrap(), &digest, allow_all()).await;

        assert!(matches!(result, Err(ApplicationError::DockerUploadTooLarge)), "{:?}", result.err());
        assert!(!blobs.exists(&digest).await.unwrap());
    }

    #[tokio::test]
    async fn a_proxied_blob_that_does_not_fit_the_quota_is_refused_and_not_cached() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"fifteen bytes!!");
        let (use_case, blobs) = proxy_use_case(proxy_repo(repository_id, Some(10)), remote);

        let result = use_case.execute_stream(repository_id, &DockerImageName::parse("library/alpine").unwrap(), &digest, allow_all()).await;

        assert!(matches!(result, Err(ApplicationError::StorageQuotaExceeded)), "{:?}", result.err());
        assert!(!blobs.exists(&digest).await.unwrap());
    }

    #[tokio::test]
    async fn concurrent_pulls_of_the_same_proxied_blob_reach_the_upstream_once() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"popular layer");
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *remote.blob_gate.lock().unwrap() = Some(gate.clone());
        let (use_case, _blobs) = proxy_use_case(proxy_repo(repository_id, None), remote.clone());
        let pull = || {
            let use_case = use_case.clone();
            let digest = digest.clone();
            tokio::spawn(async move {
                let name = DockerImageName::parse("library/alpine").unwrap();
                let stream = use_case.execute_stream(repository_id, &name, &digest, allow_all()).await.unwrap();
                read_all(stream).await
            })
        };

        let first = pull();
        while remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let second = pull();
        let third = pull();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst), 1, "the others wait for the fill in progress");

        gate.add_permits(1);
        for pulled in [first, second, third] {
            assert_eq!(pulled.await.unwrap(), Some(b"popular layer".to_vec()));
        }
        assert_eq!(remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fills_in_progress_count_against_the_quota_of_the_next_ones() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let first_layer = remote.serve_blob(b"fifteen bytes!!");
        let second_layer = remote.serve_blob(b"another fifteen");
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *remote.blob_gate.lock().unwrap() = Some(gate.clone());
        let (use_case, _blobs) = proxy_use_case(proxy_repo(repository_id, Some(20)), remote.clone());
        let name = DockerImageName::parse("library/alpine").unwrap();
        let first = {
            let use_case = use_case.clone();
            let name = name.clone();
            tokio::spawn(async move { use_case.execute_stream(repository_id, &name, &first_layer, allow_all()).await })
        };
        while remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let second = use_case.execute_stream(repository_id, &name, &second_layer, allow_all()).await;

        assert!(matches!(second, Err(ApplicationError::StorageQuotaExceeded)), "the first fill has promised 15 of the 20 bytes");
        gate.add_permits(1);
        assert!(first.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn the_first_client_gets_its_response_before_the_fill_is_done() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"a slow layer");
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *remote.blob_gate.lock().unwrap() = Some(gate.clone());
        let (use_case, _blobs) = proxy_use_case(proxy_repo(repository_id, None), remote);
        let name = DockerImageName::parse("library/alpine").unwrap();

        let stream = tokio::time::timeout(Duration::from_secs(2), use_case.execute_stream(repository_id, &name, &digest, allow_all()))
            .await
            .expect("the response must not wait for the whole fill")
            .unwrap();

        gate.add_permits(1);
        assert_eq!(read_all(stream).await, Some(b"a slow layer".to_vec()));
    }

    #[tokio::test]
    async fn a_fill_carries_on_when_the_client_goes_away() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"a layer nobody waited for");
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *remote.blob_gate.lock().unwrap() = Some(gate.clone());
        let (use_case, blobs) = proxy_use_case(proxy_repo(repository_id, None), remote.clone());
        let name = DockerImageName::parse("library/alpine").unwrap();

        let stream = use_case.execute_stream(repository_id, &name, &digest, allow_all()).await.unwrap();
        drop(stream);
        gate.add_permits(1);

        tokio::time::timeout(Duration::from_secs(2), async {
            while !blobs.is_uploaded_to_repository(repository_id, &digest).await.unwrap() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the blob must land in the cache although nobody was reading");
        let again = use_case.execute_stream(repository_id, &name, &digest, allow_all()).await.unwrap();
        assert_eq!(read_all(again).await, Some(b"a layer nobody waited for".to_vec()));
        assert_eq!(remote.blob_fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_let_go_and_the_fill_finishes_without_it() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        *remote.chunk_size.lock().unwrap() = Some(1);
        let digest = remote.serve_blob(b"a layer read by a client that stalls");
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(proxy_repo(repository_id, None));
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let use_case = GetBlobUseCase::new(blobs.clone(), Arc::new(FakeDockerManifestRepository::new()), repositories, remote).with_client_stall_limit(Duration::from_millis(100));
        let name = DockerImageName::parse("library/alpine").unwrap();

        let stream = use_case.execute_stream(repository_id, &name, &digest, allow_all()).await.unwrap().unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while !blobs.is_uploaded_to_repository(repository_id, &digest).await.unwrap() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a client that reads nothing must not hold the fill back");
        let items: Vec<_> = stream.collect().await;
        assert!(matches!(items.last(), Some(Err(_))), "what the stalled client gets ends in an error, not in what looks like a whole blob");
    }

    #[tokio::test]
    async fn a_head_on_an_uncached_proxied_blob_reports_the_remotes_length_without_caching_it() {
        let repository_id = Uuid::new_v4();
        let remote = Arc::new(FakeRemoteDockerRegistry::new());
        let digest = remote.serve_blob(b"a layer we only want the size of");
        let (use_case, blobs) = proxy_use_case(proxy_repo(repository_id, None), remote.clone());

        let size = use_case.execute_exists(repository_id, &DockerImageName::parse("library/alpine").unwrap(), &digest, allow_all()).await.unwrap();

        assert_eq!(size, Some(b"a layer we only want the size of".len() as u64));
        assert!(!blobs.exists(&digest).await.unwrap(), "a HEAD does not download the blob");
    }
}
