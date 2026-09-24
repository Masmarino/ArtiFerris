use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("storage io failure: {0}")]
    Io(String),
}

/// A boxed stream of chunks, for serving a large object without buffering it fully in memory.
pub type ByteStream = BoxStream<'static, Result<Bytes, StorageError>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeSpace {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[async_trait]
pub trait StorageBackendPort: Send + Sync {
    async fn write(&self, repository_id: Uuid, path: &str, data: &[u8]) -> Result<(), StorageError>;
    async fn read(&self, repository_id: Uuid, path: &str) -> Result<Vec<u8>, StorageError>;
    /// Same content as `read`, chunked instead of buffered whole — for a large object
    /// (an npm tarball) on the download path, where the client would otherwise force the
    /// whole file into memory before the first byte goes out.
    async fn read_stream(&self, repository_id: Uuid, path: &str) -> Result<ByteStream, StorageError>;
    async fn delete(&self, repository_id: Uuid, path: &str) -> Result<(), StorageError>;
    /// Removes everything stored under `repository_id`'s own root, not just one object at a time —
    /// used by the repository deletion sweep (B-39) to reclaim a hard-deleted repository's on-disk
    /// files (npm tarballs) once its DB rows are gone. A repository with nothing on disk (or whose
    /// directory never existed, e.g. a Docker-format repository, which never uses this backend) is a
    /// no-op, matching `delete`'s "already gone" semantics.
    async fn delete_repository(&self, repository_id: Uuid) -> Result<(), StorageError>;
    async fn used_bytes(&self, repository_id: Uuid) -> Result<u64, StorageError>;
    async fn is_healthy(&self) -> bool;
    /// Disk-level total/free — distinct from `used_bytes` (one repository's own usage).
    async fn volume_space(&self) -> Result<VolumeSpace, StorageError>;
}
