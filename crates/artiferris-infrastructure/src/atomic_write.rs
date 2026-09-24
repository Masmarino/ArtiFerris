use std::path::{Path, PathBuf};

use tokio::fs;
use uuid::Uuid;

/// Removes its file when dropped, so a write that is cancelled or fails can't leave a partial file behind.
/// Harmless once the file has been renamed away.
pub struct TempFileGuard(PathBuf);

impl TempFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// First half of `atomic_write`: creates `target`'s parent directory and writes `data` to a
/// fresh sibling `{tmp_name_prefix}.tmp-{uuid}` file, returning its path. Doesn't touch `target`
/// itself, so this can safely run without holding any lock that only guards `target` — the temp
/// path's per-call UUID suffix can't collide with any concurrent writer or deleter of the same
/// logical target. The caller must follow up with `finish_atomic_write` (or otherwise consume/
/// remove the temp file) to avoid leaking it.
pub async fn write_temp(target: &Path, tmp_name_prefix: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp_path = target.with_file_name(format!("{tmp_name_prefix}.tmp-{}", Uuid::new_v4()));
    if let Err(e) = fs::write(&tmp_path, data).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    Ok(tmp_path)
}

/// Second half of `atomic_write`: renames a temp file written by `write_temp` onto `target`
/// (atomic on the same filesystem). This is the only step that touches `target` itself, so it's
/// the only step that needs to run under a lock scoped to `target`. A failed rename removes the temp file.
pub async fn finish_atomic_write(tmp_path: &Path, target: &Path) -> std::io::Result<()> {
    let renamed = fs::rename(tmp_path, target).await;
    if renamed.is_err() {
        let _ = fs::remove_file(tmp_path).await;
    }
    renamed
}

/// Writes `data` to `target` without ever leaving a partially-written file at that path. Doesn't
/// fsync.
///
/// `tmp_name_prefix` is caller-chosen (e.g. the target's own file name, or just a digest) so
/// each caller keeps its own staging-file naming; it only has to be unique enough that two
/// concurrent writes to different targets in the same directory don't collide, which the
/// per-call UUID suffix guarantees on its own.
///
/// This convenience wrapper's only caller is `filesystem_storage.rs`'s npm storage backend.
/// `FilesystemDockerBlobStore::write` calls `write_temp` and `finish_atomic_write` directly
/// instead, to shrink its lock's critical section to just the final rename.
pub async fn atomic_write(target: &Path, tmp_name_prefix: &str, data: &[u8]) -> std::io::Result<()> {
    let tmp_path = write_temp(target, tmp_name_prefix, data).await?;
    finish_atomic_write(&tmp_path, target).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|name| name.contains(".tmp-")).collect()
    }

    #[tokio::test]
    async fn a_failed_rename_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("blob");
        std::fs::create_dir_all(target.join("occupied")).unwrap();

        assert!(atomic_write(&target, "blob", b"data").await.is_err());

        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_successful_write_leaves_only_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("blob");

        atomic_write(&target, "blob", b"data").await.unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"data");
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    }
}
