use std::path::{Path, PathBuf};

use tokio::fs;
use uuid::Uuid;

/// Removes its file when dropped, so a cancelled or failed write leaves no partial file.
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

/// First half of `atomic_write`: writes `data` to a fresh sibling `{tmp_name_prefix}.tmp-{uuid}` file and returns its
/// path. It never touches `target`, so it needs no lock on it. The caller must call `finish_atomic_write`, or remove
/// the temp file.
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

/// Second half of `atomic_write`: renames the temp file onto `target`, atomic on one filesystem. The only step that
/// needs a lock scoped to `target`. A failed rename removes the temp file.
pub async fn finish_atomic_write(tmp_path: &Path, target: &Path) -> std::io::Result<()> {
    let renamed = fs::rename(tmp_path, target).await;
    if renamed.is_err() {
        let _ = fs::remove_file(tmp_path).await;
    }
    renamed
}

/// Writes `data` to `target` without ever leaving a partial file there. Does not fsync. `tmp_name_prefix` is chosen by
/// the caller; the per-call UUID keeps concurrent writes apart. The Docker blob store calls `write_temp` and
/// `finish_atomic_write` directly to keep its lock short.
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
