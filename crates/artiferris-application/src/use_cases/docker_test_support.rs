use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use artiferris_domain::audit::{AdminAuditEvent, AuditPage, AuditQueryFilter, DockerRegistryEvent, EventPublisherPort, NpmPackageEvent, SecurityEvent};
use artiferris_domain::docker_registry::{
    BlobSweepReport, ByteStream, Digest, DockerAccessClaims, DockerBlobStorePort, DockerGrantedScope, DockerImageName, DockerManifest, DockerManifestRepositoryPort, DockerMediaType, DockerTokenIssuerPort,
    DockerUploadSession, DockerUploadSessionPort,
};
use artiferris_domain::docker_remote::{RemoteBlob, RemoteDockerRegistryPort};
use artiferris_domain::docker_scan::{DockerImageScanRepositoryPort, DockerImageScanResult, DockerImageScannerPort, DockerVulnerability};
use artiferris_domain::error::{DomainError, EventStoreError};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use uuid::Uuid;

pub struct FakeUploadSessions {
    pub sessions: Mutex<HashMap<Uuid, (DockerUploadSession, Vec<u8>)>>,
    pub sealed: Mutex<HashSet<Uuid>>,
}

impl FakeUploadSessions {
    pub fn new() -> Self {
        Self { sessions: Mutex::new(HashMap::new()), sealed: Mutex::new(HashSet::new()) }
    }
}

#[async_trait]
impl DockerUploadSessionPort for FakeUploadSessions {
    async fn create(&self, package_repository_id: Uuid) -> Result<DockerUploadSession, DomainError> {
        let mut sessions = self.sessions.lock().unwrap();
        let open = sessions.values().filter(|(s, _)| s.package_repository_id == package_repository_id).count();
        if open >= artiferris_domain::docker_registry::MAX_OPEN_UPLOADS_PER_REPOSITORY {
            return Err(DomainError::TooManyUploads);
        }
        let session = DockerUploadSession {
            id: Uuid::new_v4(),
            package_repository_id,
            staging_path: String::new(),
            bytes_received: 0,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        };
        sessions.insert(session.id, (session.clone(), Vec::new()));
        Ok(session)
    }
    async fn find(&self, id: Uuid) -> Result<Option<DockerUploadSession>, DomainError> {
        Ok(self.sessions.lock().unwrap().get(&id).map(|(s, _)| s.clone()))
    }
    async fn append_chunk(&self, id: Uuid, chunk: &[u8], expected_start: Option<i64>) -> Result<i64, DomainError> {
        let body: ByteStream = Box::pin(futures::stream::once(std::future::ready(Ok(bytes::Bytes::copy_from_slice(chunk)))));
        self.append_stream(id, body, expected_start, u64::MAX).await
    }
    async fn append_stream(&self, id: Uuid, body: ByteStream, expected_start: Option<i64>, max_bytes: u64) -> Result<i64, DomainError> {
        use futures::StreamExt;
        let mut body = body;
        let mut received = Vec::new();
        while let Some(part) = body.next().await {
            received.extend_from_slice(&part?);
            if received.len() as u64 > max_bytes {
                return Err(DomainError::UploadTooLarge);
            }
        }
        if self.sealed.lock().unwrap().contains(&id) {
            return Err(DomainError::UploadInProgress);
        }
        let mut sessions = self.sessions.lock().unwrap();
        let (session, bytes) = sessions.get_mut(&id).ok_or(DomainError::UploadSessionNotFound)?;
        if let Some(expected_start) = expected_start {
            if expected_start != session.bytes_received {
                return Err(DomainError::ChunkOffsetMismatch { expected: session.bytes_received, got: expected_start });
            }
        }
        bytes.extend_from_slice(&received);
        session.bytes_received = bytes.len() as i64;
        Ok(session.bytes_received)
    }
    async fn rewind(&self, id: Uuid, from_bytes: i64, to_bytes: i64) -> Result<(), DomainError> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some((session, bytes)) = sessions.get_mut(&id) {
            if session.bytes_received == from_bytes {
                bytes.truncate(to_bytes as usize);
                session.bytes_received = to_bytes;
            }
        }
        Ok(())
    }
    async fn seal(&self, id: Uuid) -> Result<DockerUploadSession, DomainError> {
        let session = self.sessions.lock().unwrap().get(&id).map(|(s, _)| s.clone()).ok_or(DomainError::UploadSessionNotFound)?;
        self.sealed.lock().unwrap().insert(id);
        Ok(session)
    }
    async fn hash_staged_file(&self, id: Uuid) -> Result<(Digest, u64), DomainError> {
        let bytes = self.sessions.lock().unwrap().get(&id).map(|(_, bytes)| bytes.clone()).ok_or(DomainError::UploadSessionNotFound)?;
        Ok((Digest::of(&bytes), bytes.len() as u64))
    }
    async fn delete(&self, id: Uuid) -> Result<(), DomainError> {
        self.sessions.lock().unwrap().remove(&id);
        self.sealed.lock().unwrap().remove(&id);
        Ok(())
    }
    async fn staged_bytes_for_repository(&self, package_repository_id: Uuid) -> Result<u64, DomainError> {
        Ok(self.sessions.lock().unwrap().values().filter(|(s, _)| s.package_repository_id == package_repository_id).map(|(_, bytes)| bytes.len() as u64).sum())
    }
    async fn sweep_expired_uploads(&self) -> Result<usize, DomainError> {
        let mut sessions = self.sessions.lock().unwrap();
        let now = chrono::Utc::now();
        let expired: Vec<Uuid> = sessions.iter().filter(|(_, (s, _))| s.expires_at < now).map(|(id, _)| *id).collect();
        for id in &expired {
            sessions.remove(id);
        }
        Ok(expired.len())
    }
}

pub struct FakeDockerBlobStore {
    pub blobs: Mutex<HashMap<String, (Vec<u8>, i64)>>, // digest string -> (bytes, ref_count)
    pub repository_links: Mutex<HashSet<(Uuid, String)>>,
    /// Every digest passed to `remove_reclaimed_blob_files`, in call order.
    pub removed_reclaimed_digests: Mutex<Vec<String>>,
}

impl FakeDockerBlobStore {
    pub fn new() -> Self {
        Self { blobs: Mutex::new(HashMap::new()), repository_links: Mutex::new(HashSet::new()), removed_reclaimed_digests: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl DockerBlobStorePort for FakeDockerBlobStore {
    async fn write(&self, digest: &Digest, bytes: &[u8]) -> Result<(), DomainError> {
        let mut blobs = self.blobs.lock().unwrap();
        blobs.entry(digest.as_str().to_string()).or_insert_with(|| (bytes.to_vec(), 0));
        Ok(())
    }
    async fn write_stream(&self, digest: &Digest, mut body: ByteStream, max_bytes: u64) -> Result<u64, DomainError> {
        use futures::StreamExt;
        let mut received = Vec::new();
        while let Some(chunk) = body.next().await {
            received.extend_from_slice(&chunk?);
            if received.len() as u64 > max_bytes {
                return Err(DomainError::UploadTooLarge);
            }
        }
        let computed = Digest::of(&received);
        if &computed != digest {
            return Err(DomainError::DigestMismatch { expected: digest.as_str().to_string(), computed: computed.as_str().to_string() });
        }
        let size = received.len() as u64;
        self.blobs.lock().unwrap().entry(digest.as_str().to_string()).or_insert((received, 0));
        Ok(size)
    }
    async fn adopt_staged_file(&self, digest: &Digest, _staging_path: &str, size_bytes: u64) -> Result<(), DomainError> {
        // Unlike `write`, this never receives the real content — a placeholder of the right length is enough for what this fake's callers actually check (existence and size).
        let mut blobs = self.blobs.lock().unwrap();
        blobs.entry(digest.as_str().to_string()).or_insert_with(|| (vec![0u8; size_bytes as usize], 0));
        Ok(())
    }
    async fn read(&self, digest: &Digest) -> Result<Vec<u8>, DomainError> {
        self.blobs.lock().unwrap().get(digest.as_str()).map(|(bytes, _)| bytes.clone()).ok_or_else(|| DomainError::Infrastructure("not found".into()))
    }
    async fn read_stream(&self, digest: &Digest) -> Result<artiferris_domain::docker_registry::ByteStream, DomainError> {
        let bytes = self.read(digest).await?;
        Ok(Box::pin(futures::stream::once(async move { Ok(bytes::Bytes::from(bytes)) })))
    }
    async fn link_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError> {
        self.repository_links.lock().unwrap().insert((repository_id, digest.as_str().to_string()));
        Ok(())
    }
    async fn is_uploaded_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError> {
        Ok(self.repository_links.lock().unwrap().contains(&(repository_id, digest.as_str().to_string())))
    }
    async fn unlink_from_repository_if_unreferenced(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError> {
        // Trusts the caller's own reachability check; single-threaded fakes have no concurrent push.
        self.repository_links.lock().unwrap().remove(&(repository_id, digest.as_str().to_string()));
        Ok(())
    }
    async fn exists(&self, digest: &Digest) -> Result<bool, DomainError> {
        Ok(self.blobs.lock().unwrap().contains_key(digest.as_str()))
    }
    async fn size_if_exists(&self, digest: &Digest) -> Result<Option<u64>, DomainError> {
        Ok(self.blobs.lock().unwrap().get(digest.as_str()).map(|(bytes, _)| bytes.len() as u64))
    }
    async fn existing_digests(&self, digests: &[Digest]) -> Result<HashSet<String>, DomainError> {
        let blobs = self.blobs.lock().unwrap();
        Ok(digests.iter().map(|d| d.as_str().to_string()).filter(|d| blobs.contains_key(d)).collect())
    }
    async fn sum_sizes(&self, digests: &[Digest]) -> Result<u64, DomainError> {
        let blobs = self.blobs.lock().unwrap();
        Ok(digests.iter().filter_map(|d| blobs.get(d.as_str())).map(|(bytes, _)| bytes.len() as u64).sum())
    }
    async fn increment_ref(&self, digest: &Digest) -> Result<(), DomainError> {
        if let Some((_, count)) = self.blobs.lock().unwrap().get_mut(digest.as_str()) {
            *count += 1;
        }
        Ok(())
    }
    async fn increment_ref_all(&self, digests: &[Digest]) -> Result<(), DomainError> {
        let mut blobs = self.blobs.lock().unwrap();
        for digest in digests {
            if let Some((_, count)) = blobs.get_mut(digest.as_str()) {
                *count += 1;
            }
        }
        Ok(())
    }
    async fn delete_if_unreferenced(&self, digest: &Digest) -> Result<bool, DomainError> {
        let linked = self.repository_links.lock().unwrap().iter().any(|(_, linked_digest)| linked_digest == digest.as_str());
        let mut blobs = self.blobs.lock().unwrap();
        if !linked && blobs.get(digest.as_str()).is_some_and(|(_, count)| *count <= 0) {
            blobs.remove(digest.as_str());
            return Ok(true);
        }
        Ok(false)
    }
    async fn remove_reclaimed_blob_files(&self, digests: &[Digest]) {
        self.removed_reclaimed_digests.lock().unwrap().extend(digests.iter().map(|d| d.as_str().to_string()));
    }
    async fn used_bytes_for_repository(&self, _repository_id: Uuid) -> Result<u64, DomainError> {
        // Digest-keyed only, no per-repository tracking — always reports zero pre-existing usage.
        Ok(0)
    }
    async fn used_bytes_for_repositories(&self, _repository_ids: &[Uuid]) -> Result<HashMap<Uuid, u64>, DomainError> {
        Ok(HashMap::new())
    }
    async fn sweep_unreferenced_blobs(&self, _older_than: chrono::DateTime<chrono::Utc>) -> Result<BlobSweepReport, DomainError> {
        Ok(BlobSweepReport::default())
    }
}

pub struct FakeDockerManifestRepository {
    pub manifests: Mutex<HashMap<Uuid, DockerManifest>>,
    pub manifest_blobs: Mutex<HashMap<Uuid, Vec<Digest>>>,
    pub manifest_list_members: Mutex<HashMap<Uuid, Vec<Digest>>>,
    pub tags: Mutex<HashMap<(Uuid, String, String), Uuid>>, // (repo_id, image_name, tag) -> manifest_id
    /// Every `set_tag` call in order, to tell which tag was pointed last.
    tag_updates: Mutex<Vec<(Uuid, String, String)>>,
    /// When each tag was last set, mirroring `docker_tags.updated_at`.
    tag_times: Mutex<HashMap<(Uuid, String, String), chrono::DateTime<chrono::Utc>>>,
    /// Like production, the manifest repository needs blob-store state (upload links, sizes, ref counts). Unset by
    /// default; see `link_blob_store`.
    blob_store: Mutex<Option<Arc<FakeDockerBlobStore>>>,
    /// How many times a single manifest was looked up by digest, for tests that assert a listing does not do that per tag.
    pub digest_lookups: AtomicUsize,
    pub bodies_read: AtomicUsize,
    /// How many (image, tag) pairs the capped tag listing handed back.
    pub recent_tags_returned: AtomicUsize,
}

impl FakeDockerManifestRepository {
    pub fn new() -> Self {
        Self {
            manifests: Mutex::new(HashMap::new()),
            manifest_blobs: Mutex::new(HashMap::new()),
            manifest_list_members: Mutex::new(HashMap::new()),
            tags: Mutex::new(HashMap::new()),
            tag_updates: Mutex::new(Vec::new()),
            tag_times: Mutex::new(HashMap::new()),
            blob_store: Mutex::new(None),
            digest_lookups: AtomicUsize::new(0),
            bodies_read: AtomicUsize::new(0),
            recent_tags_returned: AtomicUsize::new(0),
        }
    }

    /// Links this fake to the use case's `FakeDockerBlobStore`.
    pub fn link_blob_store(&self, blobs: Arc<FakeDockerBlobStore>) {
        *self.blob_store.lock().unwrap() = Some(blobs);
    }

    /// What the real adapter does in the transaction that deletes a manifest: each of its blobs loses one reference.
    fn release_blob_references(&self, manifest_id: Uuid) {
        let Some(blobs) = self.blob_store.lock().unwrap().clone() else { return };
        let digests = self.manifest_blobs.lock().unwrap().get(&manifest_id).cloned().unwrap_or_default();
        let mut counts = blobs.blobs.lock().unwrap();
        for digest in digests.iter().collect::<HashSet<_>>() {
            if let Some((_, count)) = counts.get_mut(digest.as_str()) {
                *count -= 1;
            }
        }
    }

    /// `set_tag` with an explicit `updated_at`, for tests that need a tag's age to differ from its manifest's.
    /// No tag points at it and no manifest list of the same repository and image lists it as a member.
    fn is_orphan(&self, manifests: &HashMap<Uuid, DockerManifest>, manifest: &DockerManifest) -> bool {
        let tagged = self.tags.lock().unwrap().values().any(|id| *id == manifest.id);
        let members = self.manifest_list_members.lock().unwrap();
        let listed = members.iter().any(|(list_id, digests)| {
            digests.contains(&manifest.digest)
                && manifests.get(list_id).is_some_and(|list| list.package_repository_id == manifest.package_repository_id && list.image_name == manifest.image_name)
        });
        !tagged && !listed
    }

    pub async fn set_tag_at(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str, manifest_id: Uuid, at: chrono::DateTime<chrono::Utc>) {
        self.set_tag(repository_id, image_name, tag, manifest_id).await.unwrap();
        self.tag_times.lock().unwrap().insert((repository_id, image_name.as_str().to_string(), tag.to_string()), at);
    }
}

#[async_trait]
impl DockerManifestRepositoryPort for FakeDockerManifestRepository {
    async fn find_manifest_by_tag(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str) -> Result<Option<DockerManifest>, DomainError> {
        let tags = self.tags.lock().unwrap();
        let Some(manifest_id) = tags.get(&(repository_id, image_name.as_str().to_string(), tag.to_string())) else { return Ok(None) };
        Ok(self.manifests.lock().unwrap().get(manifest_id).cloned())
    }
    async fn find_manifest_by_digest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest) -> Result<Option<DockerManifest>, DomainError> {
        self.digest_lookups.fetch_add(1, Ordering::Relaxed);
        Ok(self.manifests.lock().unwrap().values().find(|m| m.package_repository_id == repository_id && &m.image_name == image_name && &m.digest == digest).cloned())
    }
    async fn insert_manifest(&self, manifest: &DockerManifest, blob_digests: &[Digest]) -> Result<(Uuid, bool), DomainError> {
        let existing_id = self
            .manifests
            .lock()
            .unwrap()
            .values()
            .find(|m| m.package_repository_id == manifest.package_repository_id && m.image_name == manifest.image_name && m.digest == manifest.digest)
            .map(|m| m.id);
        if let Some(existing_id) = existing_id {
            return Ok((existing_id, false));
        }
        self.manifests.lock().unwrap().insert(manifest.id, manifest.clone());
        self.manifest_blobs.lock().unwrap().insert(manifest.id, blob_digests.to_vec());
        Ok((manifest.id, true))
    }
    async fn insert_manifest_with_checks(
        &self,
        repository_id: Uuid,
        manifest: &DockerManifest,
        blob_digests: &[Digest],
        quota_bytes: Option<i64>,
    ) -> Result<(Uuid, bool), DomainError> {
        let blob_store = self.blob_store.lock().unwrap().clone();

        for digest in blob_digests {
            let reachable_via_manifest = self.blob_is_reachable(repository_id, digest).await?;
            let reachable_via_upload = blob_store.as_ref().is_some_and(|store| store.repository_links.lock().unwrap().contains(&(repository_id, digest.as_str().to_string())));
            if !reachable_via_manifest && !reachable_via_upload {
                return Err(DomainError::DockerBlobNotReachable(digest.as_str().to_string()));
            }
        }

        if let Some(quota) = quota_bytes {
            let added_bytes: u64 = match &blob_store {
                Some(store) => {
                    let blobs = store.blobs.lock().unwrap();
                    blob_digests.iter().filter_map(|d| blobs.get(d.as_str())).map(|(bytes, _)| bytes.len() as u64).sum()
                }
                None => 0,
            };
            let used_bytes: u64 = 0;
            if used_bytes + added_bytes > quota as u64 {
                return Err(DomainError::StorageQuotaExceeded);
            }
        }

        let (manifest_id, inserted) = self.insert_manifest(manifest, blob_digests).await?;
        if inserted {
            if let Some(store) = &blob_store {
                store.increment_ref_all(blob_digests).await?;
            }
        }
        Ok((manifest_id, inserted))
    }
    async fn insert_manifest_list_members(&self, list_manifest_id: Uuid, member_digests: &[Digest]) -> Result<(), DomainError> {
        self.manifest_list_members.lock().unwrap().insert(list_manifest_id, member_digests.to_vec());
        Ok(())
    }
    async fn list_manifest_blob_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError> {
        Ok(self.manifest_blobs.lock().unwrap().get(&manifest_id).cloned().unwrap_or_default())
    }
    async fn list_manifest_list_member_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError> {
        Ok(self.manifest_list_members.lock().unwrap().get(&manifest_id).cloned().unwrap_or_default())
    }
    async fn set_tag(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str, manifest_id: Uuid) -> Result<(), DomainError> {
        let key = (repository_id, image_name.as_str().to_string(), tag.to_string());
        self.tags.lock().unwrap().insert(key.clone(), manifest_id);
        self.tag_updates.lock().unwrap().push(key.clone());
        let mut times = self.tag_times.lock().unwrap();
        let latest = times.values().max().copied().unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
        times.insert(key, chrono::Utc::now().max(latest + chrono::Duration::microseconds(1)));
        Ok(())
    }
    async fn delete_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest) -> Result<(), DomainError> {
        let manifest_id = self.manifests.lock().unwrap().values().find(|m| m.package_repository_id == repository_id && &m.image_name == image_name && &m.digest == digest).map(|m| m.id);
        if let Some(id) = manifest_id {
            self.release_blob_references(id);
            self.manifests.lock().unwrap().remove(&id);
            self.manifest_blobs.lock().unwrap().remove(&id);
            self.tags.lock().unwrap().retain(|_, v| *v != id);
        }
        Ok(())
    }
    async fn list_tags(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<String>, DomainError> {
        Ok(self.tags.lock().unwrap().keys().filter(|(rid, name, _)| *rid == repository_id && name == image_name.as_str()).map(|(_, _, tag)| tag.clone()).collect())
    }
    async fn list_repository_image_names(&self, repository_id: Uuid) -> Result<Vec<DockerImageName>, DomainError> {
        let names: Vec<String> = self.tags.lock().unwrap().keys().filter(|(rid, _, _)| *rid == repository_id).map(|(_, name, _)| name.clone()).collect();
        names.into_iter().map(|n| DockerImageName::parse(&n)).collect()
    }
    async fn list_image_names_for_repositories(&self, repository_ids: &[Uuid]) -> Result<Vec<(Uuid, DockerImageName)>, DomainError> {
        let mut pairs: Vec<(Uuid, String)> =
            self.tags.lock().unwrap().keys().filter(|(rid, _, _)| repository_ids.contains(rid)).map(|(rid, name, _)| (*rid, name.clone())).collect();
        pairs.sort();
        pairs.dedup();
        Ok(pairs.into_iter().filter_map(|(rid, name)| DockerImageName::parse(&name).ok().map(|n| (rid, n))).collect())
    }
    async fn list_image_names_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<DockerImageName>, DomainError> {
        let mut names: Vec<String> = self.tags.lock().unwrap().keys().filter(|(rid, name, _)| *rid == repository_id && after.is_none_or(|after| name.as_str() > after)).map(|(_, name, _)| name.clone()).collect();
        names.sort();
        names.dedup();
        names.truncate(limit.max(0) as usize);
        names.into_iter().map(|n| DockerImageName::parse(&n)).collect()
    }
    async fn list_recent_tags_for_images(&self, repository_id: Uuid, image_names: &[String], per_image: i64) -> Result<Vec<(DockerImageName, String)>, DomainError> {
        let tag_updates = self.tag_updates.lock().unwrap();
        let tags = self.tags.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut per_image_count: HashMap<String, i64> = HashMap::new();
        let mut pairs: Vec<(String, String)> = Vec::new();
        for (rid, name, tag) in tag_updates.iter().rev() {
            if *rid != repository_id || !image_names.contains(name) || !tags.contains_key(&(*rid, name.clone(), tag.clone())) || !seen.insert((name.clone(), tag.clone())) {
                continue;
            }
            let count = per_image_count.entry(name.clone()).or_default();
            *count += 1;
            if *count <= per_image {
                pairs.push((name.clone(), tag.clone()));
            }
        }
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        self.recent_tags_returned.fetch_add(pairs.len(), std::sync::atomic::Ordering::Relaxed);
        pairs.into_iter().map(|(name, tag)| DockerImageName::parse(&name).map(|n| (n, tag))).collect()
    }
    async fn list_latest_manifest_id_per_image(&self, repository_id: Uuid, image_names: &[String]) -> Result<Vec<(DockerImageName, Uuid)>, DomainError> {
        let tag_updates = self.tag_updates.lock().unwrap();
        let tags = self.tags.lock().unwrap();
        let mut latest: HashMap<String, Uuid> = HashMap::new();
        for (rid, name, tag) in tag_updates.iter() {
            if *rid != repository_id || !image_names.contains(name) {
                continue;
            }
            if let Some(manifest_id) = tags.get(&(*rid, name.clone(), tag.clone())) {
                latest.insert(name.clone(), *manifest_id);
            }
        }
        latest.into_iter().map(|(name, id)| DockerImageName::parse(&name).map(|n| (n, id))).collect()
    }
    async fn list_distinct_digests_for_image(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<Digest>, DomainError> {
        let manifest_ids: Vec<Uuid> =
            self.tags.lock().unwrap().iter().filter(|((rid, name, _), _)| *rid == repository_id && name == image_name.as_str()).map(|(_, id)| *id).collect();
        let manifests = self.manifests.lock().unwrap();
        let mut digests: Vec<Digest> = manifest_ids.into_iter().filter_map(|id| manifests.get(&id).map(|m| m.digest.clone())).collect();
        digests.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        digests.dedup();
        Ok(digests)
    }
    async fn list_tag_manifest_summaries(&self, repository_id: Uuid, image_name: &DockerImageName, limit: i64) -> Result<Vec<(String, Digest, DockerMediaType, chrono::DateTime<chrono::Utc>)>, DomainError> {
        let tag_updates = self.tag_updates.lock().unwrap();
        let tags = self.tags.lock().unwrap();
        let manifests = self.manifests.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut summaries = Vec::new();
        for (rid, name, tag) in tag_updates.iter().rev() {
            if *rid != repository_id || name != image_name.as_str() || !seen.insert(tag.clone()) {
                continue;
            }
            if let Some(manifest) = tags.get(&(*rid, name.clone(), tag.clone())).and_then(|id| manifests.get(id)) {
                summaries.push((tag.clone(), manifest.digest.clone(), manifest.media_type.clone(), manifest.created_at));
            }
        }
        summaries.truncate(limit.max(0) as usize);
        Ok(summaries)
    }
    async fn list_tagged_manifest_bodies(&self, repository_id: Uuid, image_name: &DockerImageName, digests: &[String], max_bytes: i64) -> Result<Vec<(Digest, Vec<u8>)>, DomainError> {
        let tags = self.tags.lock().unwrap();
        let manifests = self.manifests.lock().unwrap();
        self.bodies_read.fetch_add(digests.len(), Ordering::Relaxed);
        let mut bodies: Vec<(Digest, Vec<u8>)> = tags
            .iter()
            .filter(|((rid, name, _), _)| *rid == repository_id && name == image_name.as_str())
            .filter_map(|(_, manifest_id)| manifests.get(manifest_id))
            .filter(|m| digests.iter().any(|d| d == m.digest.as_str()) && m.body.len() as i64 <= max_bytes)
            .map(|m| (m.digest.clone(), m.body.clone()))
            .collect();
        bodies.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        bodies.dedup_by(|a, b| a.0 == b.0);
        Ok(bodies)
    }
    async fn list_repository_tag_manifest_summaries(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, DockerMediaType, chrono::DateTime<chrono::Utc>)>, DomainError> {
        let tags = self.tags.lock().unwrap();
        let manifests = self.manifests.lock().unwrap();
        let mut summaries: Vec<(DockerImageName, String, Digest, DockerMediaType, chrono::DateTime<chrono::Utc>)> = tags
            .iter()
            .filter(|((rid, _, _), _)| *rid == repository_id)
            .filter_map(|((_, name, tag), manifest_id)| {
                let manifest = manifests.get(manifest_id)?;
                Some((DockerImageName::parse(name).ok()?, tag.clone(), manifest.digest.clone(), manifest.media_type.clone(), manifest.created_at))
            })
            .collect();
        summaries.sort_by(|a, b| (a.0.as_str(), &a.1).cmp(&(b.0.as_str(), &b.1)));
        Ok(summaries)
    }

    async fn list_repository_tag_updates(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, chrono::DateTime<chrono::Utc>)>, DomainError> {
        let tags = self.tags.lock().unwrap();
        let times = self.tag_times.lock().unwrap();
        let manifests = self.manifests.lock().unwrap();
        let mut updates: Vec<(DockerImageName, String, Digest, chrono::DateTime<chrono::Utc>)> = tags
            .iter()
            .filter(|((rid, _, _), _)| *rid == repository_id)
            .filter_map(|(key @ (_, name, tag), manifest_id)| {
                let manifest = manifests.get(manifest_id)?;
                Some((DockerImageName::parse(name).ok()?, tag.clone(), manifest.digest.clone(), *times.get(key)?))
            })
            .collect();
        updates.sort_by(|a, b| (a.0.as_str(), &a.1).cmp(&(b.0.as_str(), &b.1)));
        Ok(updates)
    }
    async fn list_untagged_manifests(&self, repository_id: Uuid, untagged_before: chrono::DateTime<chrono::Utc>) -> Result<Vec<(DockerImageName, Digest)>, DomainError> {
        let manifests = self.manifests.lock().unwrap();
        Ok(manifests.values().filter(|m| m.package_repository_id == repository_id && m.created_at < untagged_before && self.is_orphan(&manifests, m)).map(|m| (m.image_name.clone(), m.digest.clone())).collect())
    }
    async fn delete_untagged_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest, untagged_before: chrono::DateTime<chrono::Utc>) -> Result<bool, DomainError> {
        let id = {
            let manifests = self.manifests.lock().unwrap();
            manifests
                .values()
                .find(|m| m.package_repository_id == repository_id && &m.image_name == image_name && &m.digest == digest && m.created_at < untagged_before && self.is_orphan(&manifests, m))
                .map(|m| m.id)
        };
        let Some(id) = id else { return Ok(false) };
        self.release_blob_references(id);
        self.manifests.lock().unwrap().remove(&id);
        self.manifest_blobs.lock().unwrap().remove(&id);
        self.manifest_list_members.lock().unwrap().remove(&id);
        Ok(true)
    }

    async fn blob_is_reachable(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError> {
        let manifests = self.manifests.lock().unwrap();
        let manifest_blobs = self.manifest_blobs.lock().unwrap();
        Ok(manifests.values().any(|m| m.package_repository_id == repository_id && manifest_blobs.get(&m.id).is_some_and(|blobs| blobs.contains(digest))))
    }
}

pub struct FakeDockerEvents {
    pub docker_events: Mutex<Vec<(DockerRegistryEvent, Uuid, Option<Uuid>)>>,
}

impl FakeDockerEvents {
    pub fn new() -> Self {
        Self { docker_events: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl EventPublisherPort for FakeDockerEvents {
    async fn publish_security_event(&self, _event: SecurityEvent, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> { Ok(()) }
    async fn publish_admin_event(&self, _event: AdminAuditEvent, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> { Ok(()) }
    async fn query_audit_log(&self, _filter: AuditQueryFilter) -> Result<AuditPage, EventStoreError> { Ok(AuditPage { entries: vec![], next_cursor: None }) }
    async fn publish_npm_event(&self, _event: NpmPackageEvent, _npm_package_id: Uuid, _package_repository_id: Uuid, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> { Ok(()) }
    async fn publish_docker_event(&self, event: DockerRegistryEvent, package_repository_id: Uuid, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        self.docker_events.lock().unwrap().push((event, package_repository_id, actor_id));
        Ok(())
    }
}

pub struct FakeRepositories {
    pub repos: Mutex<HashMap<Uuid, PackageRepositorySummary>>,
}

impl FakeRepositories {
    pub fn new() -> Self {
        Self { repos: Mutex::new(HashMap::new()) }
    }

    pub fn insert(&self, repo: PackageRepositorySummary) {
        self.repos.lock().unwrap().insert(repo.id, repo);
    }
}

#[async_trait]
impl PackageRepositoryQueryPort for FakeRepositories {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.lock().unwrap().get(&id).cloned())
    }
    async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.lock().unwrap().values().find(|r| r.organization_id == organization_id && r.name == name).cloned())
    }
    async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.lock().unwrap().values().cloned().collect())
    }
    async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.lock().unwrap().values().filter(|r| r.organization_id == organization_id).cloned().collect())
    }
}

pub struct FakeDockerTokenIssuer {
    pub issued: Mutex<Vec<(Uuid, Option<DockerGrantedScope>)>>,
}

impl FakeDockerTokenIssuer {
    pub fn new() -> Self {
        Self { issued: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl DockerTokenIssuerPort for FakeDockerTokenIssuer {
    fn issue(&self, user_id: Uuid, _organization_id: Uuid, _is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
        self.issued.lock().unwrap().push((user_id, granted_scope));
        Ok("fake-registry-token".to_string())
    }
    fn issue_for_api_token(&self, _api_token_id: Uuid, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
        self.issue(user_id, organization_id, is_super_admin, granted_scope)
    }
    fn verify(&self, _token: &str) -> Result<DockerAccessClaims, DomainError> {
        unreachable!("not exercised by ScanDockerImageUseCase's tests")
    }
}

/// Returns a fixed list of vulnerabilities and records the arguments it was scanned with.
pub struct FakeDockerImageScanner {
    pub vulnerabilities: Vec<DockerVulnerability>,
    pub last_call: Mutex<Option<(String, String, String, Option<String>, String)>>,
    /// When set, a scan waits for a permit before it starts, like a scan queued behind the concurrency limit.
    pub slot_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
}

impl FakeDockerImageScanner {
    pub fn new(vulnerabilities: Vec<DockerVulnerability>) -> Self {
        Self { vulnerabilities, last_call: Mutex::new(None), slot_gate: Mutex::new(None) }
    }
}

#[async_trait]
impl DockerImageScannerPort for FakeDockerImageScanner {
    async fn scan(
        &self,
        repository_name: &str,
        image_name: &str,
        reference: &str,
        platform: Option<&str>,
        mint_registry_token: &(dyn Fn() -> Result<String, DomainError> + Send + Sync),
    ) -> Result<Vec<DockerVulnerability>, DomainError> {
        let gate = self.slot_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        let registry_token = mint_registry_token()?;
        *self.last_call.lock().unwrap() =
            Some((repository_name.to_string(), image_name.to_string(), reference.to_string(), platform.map(str::to_string), registry_token));
        Ok(self.vulnerabilities.clone())
    }
}

/// Mirrors the Postgres adapter's "insert-only, latest wins by `scanned_at`" semantics.
pub struct FakeDockerImageScanResults {
    pub saved: Mutex<Vec<DockerImageScanResult>>,
}

impl FakeDockerImageScanResults {
    pub fn new() -> Self {
        Self { saved: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl DockerImageScanRepositoryPort for FakeDockerImageScanResults {
    async fn save(&self, result: &DockerImageScanResult) -> Result<(), DomainError> {
        self.saved.lock().unwrap().push(result.clone());
        Ok(())
    }
    async fn find_latest_for_manifest(&self, docker_manifest_id: Uuid) -> Result<Option<DockerImageScanResult>, DomainError> {
        Ok(self.saved.lock().unwrap().iter().filter(|r| r.docker_manifest_id == docker_manifest_id).max_by_key(|r| r.scanned_at).cloned())
    }
    async fn find_latest_for_manifests(&self, docker_manifest_ids: &[Uuid]) -> Result<Vec<DockerImageScanResult>, DomainError> {
        let saved = self.saved.lock().unwrap();
        Ok(docker_manifest_ids
            .iter()
            .filter_map(|id| saved.iter().filter(|r| r.docker_manifest_id == *id).max_by_key(|r| r.scanned_at).cloned())
            .collect())
    }
}

/// `manifest_response == None` simulates a real 404 (`Ok(None)`), not a fetch error.
pub struct FakeRemoteDockerRegistry {
    pub manifest_response: Mutex<Option<(Vec<u8>, String)>>,
    /// What every blob fetch answers with, unless `served_blobs` has the digest; `None` is a 404.
    pub blob_response: Mutex<Option<Vec<u8>>>,
    /// Blobs answered by digest, keyed by the digest of their content.
    pub served_blobs: Mutex<HashMap<String, Vec<u8>>>,
    pub blob_fetches: AtomicUsize,
    /// When set, the body of a blob fetch waits for a permit before it arrives.
    pub blob_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// Woken each time a blob body starts being read.
    pub blob_fetch_started: Arc<tokio::sync::Notify>,
    /// When set, a blob body arrives in chunks of this many bytes rather than in one.
    pub chunk_size: Mutex<Option<usize>>,
}

impl FakeRemoteDockerRegistry {
    /// Makes `bytes` the answer for its own digest.
    pub fn serve_blob(&self, bytes: &[u8]) -> Digest {
        let digest = Digest::of(bytes);
        self.served_blobs.lock().unwrap().insert(digest.as_str().to_string(), bytes.to_vec());
        digest
    }

    pub fn new() -> Self {
        Self {
            manifest_response: Mutex::new(Some((b"{}".to_vec(), "application/vnd.docker.distribution.manifest.v2+json".to_string()))),
            blob_response: Mutex::new(Some(b"fake-blob-bytes".to_vec())),
            served_blobs: Mutex::new(HashMap::new()),
            blob_fetches: AtomicUsize::new(0),
            blob_gate: Mutex::new(None),
            blob_fetch_started: Arc::new(tokio::sync::Notify::new()),
            chunk_size: Mutex::new(None),
        }
    }
}

#[async_trait]
impl RemoteDockerRegistryPort for FakeRemoteDockerRegistry {
    async fn fetch_manifest(
        &self,
        _base_url: &str,
        _image_name: &DockerImageName,
        _reference: &str,
        _username: Option<&str>,
        _password: Option<&str>,
    ) -> Result<Option<(Vec<u8>, String)>, DomainError> {
        Ok(self.manifest_response.lock().unwrap().clone())
    }
    async fn fetch_blob(
        &self,
        _base_url: &str,
        _image_name: &DockerImageName,
        digest: &Digest,
        _username: Option<&str>,
        _password: Option<&str>,
    ) -> Result<Option<RemoteBlob>, DomainError> {
        self.blob_fetches.fetch_add(1, Ordering::SeqCst);
        let served = self.served_blobs.lock().unwrap().get(digest.as_str()).cloned();
        let Some(bytes) = served.or_else(|| self.blob_response.lock().unwrap().clone()) else { return Ok(None) };
        let length = bytes.len() as u64;
        let gate = self.blob_gate.lock().unwrap().clone();
        let started = self.blob_fetch_started.clone();
        let chunk_size = *self.chunk_size.lock().unwrap();
        let body = futures::StreamExt::flatten(futures::stream::once(async move {
            started.notify_waiters();
            if let Some(gate) = gate {
                let _ = gate.acquire().await;
            }
            let chunks: Vec<Result<bytes::Bytes, DomainError>> = match chunk_size {
                Some(size) => bytes.chunks(size).map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk))).collect(),
                None => vec![Ok(bytes::Bytes::from(bytes))],
            };
            futures::stream::iter(chunks)
        }));
        Ok(Some(RemoteBlob { content_length: Some(length), stream: Box::pin(body) }))
    }
}
