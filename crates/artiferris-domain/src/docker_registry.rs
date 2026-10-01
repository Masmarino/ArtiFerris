use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_core::stream::BoxStream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

/// The largest blob the registry takes in, pushed by a client or fetched for a proxy cache.
pub const MAX_BLOB_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// A boxed stream of chunks, for serving a large blob without buffering it fully in memory.
pub type ByteStream = BoxStream<'static, Result<Bytes, DomainError>>;

/// Its claims (role, organization) are a snapshot from issuance, so this is how long a role change can go unnoticed.
pub const DOCKER_ACCESS_TOKEN_TTL_SECONDS: i64 = 120;

pub const MAX_OPEN_UPLOADS_PER_REPOSITORY: usize = 32;

pub const MAX_TAGS_PER_REPOSITORY: i64 = 10_000;

/// What each tag counts for against a repository's quota, on top of the manifest bodies and blobs.
pub const DOCKER_TAG_QUOTA_BYTES: i64 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest(String);

impl Digest {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let Some(hex) = raw.strip_prefix("sha256:") else {
            return Err(DomainError::Validation(format!("unsupported digest algorithm: {raw}")));
        };
        let valid = hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if valid {
            Ok(Self(raw.to_string()))
        } else {
            Err(DomainError::Validation(format!("invalid sha256 digest: {raw}")))
        }
    }

    /// The real digest of `bytes`; never trust a client-declared one.
    pub fn of(bytes: &[u8]) -> Self {
        use sha2::{Digest as _, Sha256};
        let hash = Sha256::digest(bytes);
        Self(format!("sha256:{}", hex::encode(hash)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DockerImageName(String);

impl DockerImageName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        if raw.is_empty() {
            return Err(DomainError::Validation("image name must not be empty".into()));
        }
        for segment in raw.split('/') {
            let valid_segment = !segment.is_empty()
                && segment.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && segment.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-');
            if !valid_segment {
                return Err(DomainError::Validation(format!("invalid image name segment: {segment:?} in {raw:?}")));
            }
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// OCI tag grammar: `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
pub fn parse_docker_tag(raw: &str) -> Result<String, DomainError> {
    let len_ok = (1..=128).contains(&raw.len());
    let starts_ok = raw.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let chars_ok = raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
    if len_ok && starts_ok && chars_ok {
        Ok(raw.to_string())
    } else {
        Err(DomainError::Validation(format!("invalid tag: {raw}")))
    }
}

/// A manifest reference is a digest or a tag, nothing else may reach a URL or a query.
pub fn validate_manifest_reference(raw: &str) -> Result<(), DomainError> {
    if Digest::parse(raw).is_ok() {
        return Ok(());
    }
    parse_docker_tag(raw).map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockerMediaType {
    DockerV2Manifest,
    OciManifest,
    OciIndex,
}

impl DockerMediaType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DockerMediaType::DockerV2Manifest => "application/vnd.docker.distribution.manifest.v2+json",
            DockerMediaType::OciManifest => "application/vnd.oci.image.manifest.v1+json",
            DockerMediaType::OciIndex => "application/vnd.oci.image.index.v1+json",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "application/vnd.docker.distribution.manifest.v2+json" => Ok(DockerMediaType::DockerV2Manifest),
            "application/vnd.oci.image.manifest.v1+json" => Ok(DockerMediaType::OciManifest),
            "application/vnd.oci.image.index.v1+json" | "application/vnd.docker.distribution.manifest.list.v2+json" => {
                Ok(DockerMediaType::OciIndex)
            }
            other => Err(DomainError::Validation(format!("unsupported manifest media type: {other}"))),
        }
    }

    pub fn is_index(&self) -> bool {
        matches!(self, DockerMediaType::OciIndex)
    }
}

#[derive(Debug, Clone)]
pub struct DockerBlob {
    pub digest: Digest,
    pub size_bytes: i64,
    pub storage_key: String,
    pub reference_count: i64,
}

#[derive(Debug, Clone)]
pub struct DockerManifest {
    pub id: Uuid,
    pub package_repository_id: Uuid,
    pub image_name: DockerImageName,
    pub digest: Digest,
    pub media_type: DockerMediaType,
    /// Exact bytes as received, never reserialized: a client re-verifies `digest` against them.
    pub body: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DockerTag {
    pub package_repository_id: Uuid,
    pub image_name: DockerImageName,
    pub tag: String,
    pub manifest_id: Uuid,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait DockerBlobStorePort: Send + Sync {
    /// Idempotent. Does not touch `reference_count` — see `increment_ref`.
    async fn write(&self, digest: &Digest, bytes: &[u8]) -> Result<(), DomainError>;
    /// Streams `body` to disk while hashing and stores it under `digest`, returning its size. Nothing is kept on
    /// `DigestMismatch` or past `max_bytes` (`UploadTooLarge`). Does not touch `reference_count`.
    async fn write_stream(&self, digest: &Digest, body: ByteStream, max_bytes: u64) -> Result<u64, DomainError>;
    /// Adopts a file already staged at `staging_path` by renaming it, instead of writing it a second time.
    async fn adopt_staged_file(&self, digest: &Digest, staging_path: &str, size_bytes: u64) -> Result<(), DomainError>;
    async fn read(&self, digest: &Digest) -> Result<Vec<u8>, DomainError>;
    /// Same content as `read`, chunked so a large layer is not buffered whole.
    async fn read_stream(&self, digest: &Digest) -> Result<ByteStream, DomainError>;
    /// Records that `digest` was uploaded to `repository_id`, independent of any manifest referencing it yet.
    async fn link_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError>;
    async fn is_uploaded_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError>;
    /// Removes the `link_to_repository` row once it is stale. Call it only after confirming (e.g. with
    /// `blob_is_reachable`) that no manifest of `repository_id` references `digest`: left in place, the row stops
    /// `decrement_ref_and_delete_if_zero` from reclaiming the blob. Implementations re-check that condition themselves,
    /// against a concurrent push.
    async fn unlink_from_repository_if_unreferenced(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError>;
    async fn exists(&self, digest: &Digest) -> Result<bool, DomainError>;
    async fn size_if_exists(&self, digest: &Digest) -> Result<Option<u64>, DomainError>;
    async fn existing_digests(&self, digests: &[Digest]) -> Result<std::collections::HashSet<String>, DomainError>;
    async fn sum_sizes(&self, digests: &[Digest]) -> Result<u64, DomainError>;
    async fn increment_ref(&self, digest: &Digest) -> Result<(), DomainError>;
    /// Batched form of `increment_ref` — callers must pass already-deduplicated digests.
    async fn increment_ref_all(&self, digests: &[Digest]) -> Result<(), DomainError>;
    /// Deletes the blob's row and file if `reference_count` is zero and nothing links it. `Ok(true)` iff the row went;
    /// a failure removing the file is logged, not returned.
    async fn delete_if_unreferenced(&self, digest: &Digest) -> Result<bool, DomainError>;
    /// Best effort: for each digest whose row the repository deletion sweep already deleted, re-takes the advisory
    /// lock, re-checks the row is still absent, then removes the file. A stray file is recoverable, a ghost row is not.
    async fn remove_reclaimed_blob_files(&self, digests: &[Digest]);
    /// Distinct blobs its manifests reference plus uploaded ones not yet referenced, the manifest bodies, and
    /// `DOCKER_TAG_QUOTA_BYTES` per tag. Blobs are deduped by digest, so one can count toward several repositories.
    async fn used_bytes_for_repository(&self, repository_id: Uuid) -> Result<u64, DomainError>;
    /// Batched `used_bytes_for_repository`. A repository with no blobs is absent, not zero.
    async fn used_bytes_for_repositories(&self, repository_ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, u64>, DomainError>;
    /// Recomputes `reference_count` from the manifests for blobs older than `older_than`, drops older link rows no
    /// hosted manifest references, then deletes blobs nothing references, files included. Newer ones may still await
    /// their manifest. Also removes half-written files.
    async fn sweep_unreferenced_blobs(&self, older_than: DateTime<Utc>) -> Result<BlobSweepReport, DomainError>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlobSweepReport {
    pub links_removed: usize,
    pub blobs_removed: usize,
    pub counts_corrected: usize,
    pub temp_files_removed: usize,
}

#[derive(Debug, Clone)]
pub struct DockerUploadSession {
    pub id: Uuid,
    pub package_repository_id: Uuid,
    pub staging_path: String,
    pub bytes_received: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait DockerUploadSessionPort: Send + Sync {
    /// A session with no chunk for an hour expires. An abandoned one is also cleaned lazily on the next `find`, which
    /// never comes if the client crashed, hence the sweep. `TooManyUploads` once the repository has
    /// `MAX_OPEN_UPLOADS_PER_REPOSITORY` sessions open.
    async fn create(&self, package_repository_id: Uuid) -> Result<DockerUploadSession, DomainError>;
    /// `None` for an expired session even if its row exists: callers cannot tell expired from never existed.
    async fn find(&self, id: Uuid) -> Result<Option<DockerUploadSession>, DomainError>;
    /// Checks `expected_start` and applies the chunk as one atomic step, if given.
    async fn append_chunk(&self, id: Uuid, chunk: &[u8], expected_start: Option<i64>) -> Result<i64, DomainError>;
    /// `append_chunk` for a body staged to a file as it arrives. Nothing is kept unless the whole stream lands: an
    /// error or more than `max_bytes` (`UploadTooLarge`) leaves the session as it was. A sealed session fails with
    /// `UploadInProgress`.
    async fn append_stream(&self, id: Uuid, chunk: ByteStream, expected_start: Option<i64>, max_bytes: u64) -> Result<i64, DomainError>;
    /// Undoes a chunk that took the session from `to_bytes` to `from_bytes`. Does nothing if the session has moved on since.
    async fn rewind(&self, id: Uuid, from_bytes: i64, to_bytes: i64) -> Result<(), DomainError>;
    /// Closes the session to further chunks, so what gets hashed next is what gets stored. Idempotent.
    async fn seal(&self, id: Uuid) -> Result<DockerUploadSession, DomainError>;
    /// Streams the staged content from disk to hash it — a chunked upload can be gigabytes.
    async fn hash_staged_file(&self, id: Uuid) -> Result<(Digest, u64), DomainError>;
    async fn delete(&self, id: Uuid) -> Result<(), DomainError>;
    async fn staged_bytes_for_repository(&self, package_repository_id: Uuid) -> Result<u64, DomainError>;
    /// Deletes every session past `expires_at`, staging file included; returns the count. Called by a background sweep.
    async fn sweep_expired_uploads(&self) -> Result<usize, DomainError>;
}

#[async_trait]
pub trait DockerManifestRepositoryPort: Send + Sync {
    async fn find_manifest_by_tag(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        tag: &str,
    ) -> Result<Option<DockerManifest>, DomainError>;
    async fn find_manifest_by_digest(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        digest: &Digest,
    ) -> Result<Option<DockerManifest>, DomainError>;
    /// Returns the row id and whether it was a real insert (false on an idempotent conflict).
    async fn insert_manifest(&self, manifest: &DockerManifest, blob_digests: &[Digest]) -> Result<(Uuid, bool), DomainError>;
    /// One transaction under an advisory lock scoped to `repository_id` (pushes to other repositories are not blocked):
    /// re-verifies every blob in `blob_digests` is reachable from this repository and the repository's `quota_bytes`
    /// against current usage plus these blobs, inserts the manifest and its blob links and, on a real insert,
    /// increments each blob's reference count. This closes the races of a check-then-insert: two pushes both reading
    /// "under quota", and a manifest inserted against a blob a concurrent delete removed. Fails with
    /// `DockerBlobNotReachable` or `StorageQuotaExceeded` instead of inserting.
    async fn insert_manifest_with_checks(
        &self,
        repository_id: Uuid,
        manifest: &DockerManifest,
        blob_digests: &[Digest],
        quota_bytes: Option<i64>,
    ) -> Result<(Uuid, bool), DomainError>;
    async fn insert_manifest_list_members(&self, list_manifest_id: Uuid, member_digests: &[Digest]) -> Result<(), DomainError>;
    async fn list_manifest_blob_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError>;
    async fn list_manifest_list_member_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError>;
    async fn set_tag(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str, manifest_id: Uuid) -> Result<(), DomainError>;
    /// Deletes the manifest and gives up its blob references in one transaction, so a failure leaves the counts as they were.
    async fn delete_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest) -> Result<(), DomainError>;
    async fn list_tags(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<String>, DomainError>;
    async fn list_repository_image_names(&self, repository_id: Uuid) -> Result<Vec<DockerImageName>, DomainError>;
    /// `list_repository_image_names` across several repositories in one query.
    async fn list_image_names_for_repositories(&self, repository_ids: &[Uuid]) -> Result<Vec<(Uuid, DockerImageName)>, DomainError>;
    /// Up to `limit` distinct image names in name order, starting after `after`.
    async fn list_image_names_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<DockerImageName>, DomainError>;
    /// The (image name, tag) pairs of the given images, at most the `per_image` most recently updated tags of each (cut
    /// in the query), ordered by image name then most recent first.
    async fn list_recent_tags_for_images(&self, repository_id: Uuid, image_names: &[String], per_image: i64) -> Result<Vec<(DockerImageName, String)>, DomainError>;
    /// The manifest behind each image's most recently updated tag, one row per image name.
    async fn list_latest_manifest_id_per_image(&self, repository_id: Uuid, image_names: &[String]) -> Result<Vec<(DockerImageName, Uuid)>, DomainError>;
    /// Distinct digests tagged anywhere on this image, in one query.
    async fn list_distinct_digests_for_image(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<Digest>, DomainError>;
    /// The `limit` most recently updated tags of the image, each joined to its manifest's digest/media type/created_at, in one query.
    async fn list_tag_manifest_summaries(&self, repository_id: Uuid, image_name: &DockerImageName, limit: i64) -> Result<Vec<(String, Digest, DockerMediaType, DateTime<Utc>)>, DomainError>;
    /// The body of each of these manifests of the image, one query. Bodies over `max_bytes` are left out.
    async fn list_tagged_manifest_bodies(&self, repository_id: Uuid, image_name: &DockerImageName, digests: &[String], max_bytes: i64) -> Result<Vec<(Digest, Vec<u8>)>, DomainError>;
    /// Every image's tag/manifest summaries in one repository, in one query.
    async fn list_repository_tag_manifest_summaries(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, DockerMediaType, DateTime<Utc>)>, DomainError>;
    /// Every tag with when the tag itself was last set: retention's order, since re-tagging an old digest keeps its own timestamp.
    async fn list_repository_tag_updates(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, DateTime<Utc>)>, DomainError>;
    /// Manifests that no tag points at and no manifest list of the same image names as a member, and that have been that way since
    /// before `untagged_before`: from when their last tag moved away, or from their push if they never had one.
    async fn list_untagged_manifests(&self, repository_id: Uuid, untagged_before: DateTime<Utc>) -> Result<Vec<(DockerImageName, Digest)>, DomainError>;
    /// `delete_manifest`, but only while the manifest is still untagged, unlisted and has been for longer than `untagged_before`. Returns whether it deleted.
    async fn delete_untagged_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest, untagged_before: DateTime<Utc>) -> Result<bool, DomainError>;
    /// Whether `digest` is referenced by a manifest actually stored in `repository_id` — blob storage is globally deduped, but reads must still be scoped per repository.
    async fn blob_is_reachable(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerScopeRequest {
    pub resource_type: String,
    pub name: String,
    pub actions: Vec<String>,
}

impl DockerScopeRequest {
    /// Parses e.g. `"repository:myrepo/myimage:pull,push"`; malformed input returns `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, ':');
        let resource_type = parts.next()?.to_string();
        let name = parts.next()?.to_string();
        let actions_part = parts.next()?;
        if resource_type.is_empty() || name.is_empty() || actions_part.is_empty() {
            return None;
        }
        Some(Self { resource_type, name, actions: actions_part.split(',').map(|s| s.to_string()).collect() })
    }

    pub fn artiferris_repository_name(&self) -> &str {
        self.name.split('/').next().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerGrantedScope {
    pub resource_type: String,
    pub name: String,
    pub actions: Vec<String>,
    /// The repository this scope was granted against. `None` only for tokens issued before this field existed.
    pub granted_repository_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct DockerAccessClaims {
    pub user_id: Uuid,
    /// Snapshot as of token issuance, not re-checked live — used by `require_same_organization` in `artiferris-docker` to reject a mismatched resolved organization.
    pub organization_id: Uuid,
    pub is_super_admin: bool,
    pub granted_scope: Option<DockerGrantedScope>,
    /// When the token was minted (second granularity, from the JWT `iat`): compare it against a user's
    /// `tokens_valid_after` truncated the same way.
    pub issued_at: DateTime<Utc>,
    /// The API token this one was exchanged for; `None` for a token the server minted for itself.
    pub api_token_id: Option<Uuid>,
}

#[async_trait]
pub trait DockerTokenIssuerPort: Send + Sync {
    fn issue(&self, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError>;
    /// Remembers the API token, so revoking it can end this one too.
    fn issue_for_api_token(&self, api_token_id: Uuid, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError>;
    fn verify(&self, token: &str) -> Result<DockerAccessClaims, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_sha256_digest() {
        let digest = Digest::parse("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap();
        assert_eq!(digest.as_str(), "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn rejects_a_digest_with_the_wrong_hex_length() {
        assert!(Digest::parse("sha256:abc123").is_err());
    }

    #[test]
    fn rejects_a_digest_with_an_unsupported_algorithm() {
        assert!(Digest::parse("md5:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").is_err());
    }

    #[test]
    fn accepts_a_simple_image_name() {
        assert!(DockerImageName::parse("nginx").is_ok());
    }

    #[test]
    fn accepts_a_namespaced_image_name() {
        let name = DockerImageName::parse("library/nginx").unwrap();
        assert_eq!(name.as_str(), "library/nginx");
    }

    #[test]
    fn accepts_a_deeply_namespaced_image_name() {
        assert!(DockerImageName::parse("myorg/team/image").is_ok());
    }

    #[test]
    fn rejects_an_uppercase_image_name() {
        assert!(DockerImageName::parse("MyImage").is_err());
    }

    #[test]
    fn rejects_an_empty_image_name() {
        assert!(DockerImageName::parse("").is_err());
    }

    #[test]
    fn a_tag_longer_than_128_characters_is_rejected() {
        let tag = "a".repeat(129);
        assert!(parse_docker_tag(&tag).is_err());
    }

    #[test]
    fn a_tag_of_exactly_128_characters_is_accepted() {
        let tag = "a".repeat(128);
        assert!(parse_docker_tag(&tag).is_ok());
    }

    #[test]
    fn a_tag_starting_with_a_dot_is_rejected() {
        assert!(parse_docker_tag(".latest").is_err());
    }

    #[test]
    fn a_tag_with_a_disallowed_character_is_rejected() {
        assert!(parse_docker_tag("latest!").is_err());
    }

    #[test]
    fn a_manifest_reference_is_a_digest_or_a_tag_and_nothing_else() {
        assert!(validate_manifest_reference("latest").is_ok());
        assert!(validate_manifest_reference(&format!("sha256:{}", "a".repeat(64))).is_ok());
        for bad in ["../../v2/other/manifests/latest", "a?b", "a#b", "a/b", "sha256:xyz", "", "%2e%2e", "a b"] {
            assert!(validate_manifest_reference(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_well_formed_tag_is_accepted() {
        assert!(parse_docker_tag("v1.2.3-alpha_1").is_ok());
    }

    #[test]
    fn a_single_upload_may_exceed_two_gib() {
        assert!(MAX_BLOB_BYTES > 2 * 1024 * 1024 * 1024);
    }
}
