#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Filesystem `ArtifactStore` for tests and local development.
//!
//! Storage model:
//! - content-addressed blobs keyed by `ArtifactId` (SHA-256 hex)
//! - `put()` computes the id from bytes
//! - `get()` verifies the hash and returns `StorageError::Corruption` on mismatch
//!
//! # Examples
//!
//! ```rust
//! use mfm_artifact_store_fs::FsArtifactStore;
//!
//! let _store = FsArtifactStore::new("/tmp/mfm-artifacts");
//! ```

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::{ArtifactId, ErrorCode};
use mfm_machine::stores::{ArtifactKind, ArtifactStore};
use tokio::io::AsyncWriteExt;

/// Filesystem-backed immutable artifact store rooted at a directory path.
#[derive(Clone, Debug)]
pub struct FsArtifactStore {
    root: PathBuf,
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FsArtifactStore {
    /// Creates a store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, id: &ArtifactId) -> PathBuf {
        let prefix = id.as_str().get(0..2).unwrap_or("xx");
        self.root.join(prefix).join(id.as_str())
    }

    fn info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode::must_new(code),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.into(),
            details: None,
        }
    }

    fn not_found(message: impl Into<String>) -> StorageError {
        StorageError::NotFound(Self::info("artifact_not_found", message))
    }

    fn corruption(message: impl Into<String>) -> StorageError {
        StorageError::Corruption(Self::info("artifact_corruption", message))
    }

    fn other(message: impl Into<String>) -> StorageError {
        StorageError::Other(Self::info("artifact_store_io", message))
    }

    async fn ensure_parent_dir(path: &Path) -> Result<(), StorageError> {
        let Some(parent) = path.parent() else {
            return Err(Self::other("artifact path had no parent directory"));
        };
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Self::other(format!("failed to create artifact directory: {e}")))
    }

    async fn read_existing(path: &Path) -> Result<Option<Vec<u8>>, StorageError> {
        match tokio::fs::read(path).await {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Self::other(format!("failed to read artifact: {e}"))),
        }
    }

    fn temp_path_for(path: &Path) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(path.file_name().unwrap_or_else(|| OsStr::new("artifact")));
        name.push(format!(".tmp-{}-{nanos}-{seq}", std::process::id()));
        path.with_file_name(name)
    }
}

#[async_trait]
impl ArtifactStore for FsArtifactStore {
    async fn put(&self, _kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        let id = artifact_id_for_bytes(&bytes);
        let path = self.path_for(&id);

        Self::ensure_parent_dir(&path).await?;

        if let Some(existing) = Self::read_existing(&path).await? {
            let existing_id = artifact_id_for_bytes(&existing);
            if existing_id != id {
                return Err(Self::corruption(
                    "artifact exists on disk but its contents do not match its id",
                ));
            }
            return Ok(id);
        }

        // Write to a private temp file first so readers never observe partial bytes.
        let temp_path = Self::temp_path_for(&path);
        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Self::other(format!(
                    "failed to reserve temp artifact path: {e}"
                )))
            }
            Err(e) => return Err(Self::other(format!("failed to create temp artifact: {e}"))),
        };

        if let Err(e) = file.write_all(&bytes).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(Self::other(format!("failed to write temp artifact: {e}")));
        }
        if let Err(e) = file.sync_all().await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(Self::other(format!("failed to sync temp artifact: {e}")));
        }
        drop(file);

        match tokio::fs::hard_link(&temp_path, &path).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                if let Some(existing) = Self::read_existing(&path).await? {
                    let existing_id = artifact_id_for_bytes(&existing);
                    if existing_id != id {
                        return Err(Self::corruption(
                            "artifact exists on disk but its contents do not match its id",
                        ));
                    }
                    return Ok(id);
                }
                return Err(Self::other(
                    "artifact appeared as existing, then disappeared during put",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::Unsupported => {
                let mut dest = match tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .await
                {
                    Ok(f) => f,
                    Err(create_err) if create_err.kind() == io::ErrorKind::AlreadyExists => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        if let Some(existing) = Self::read_existing(&path).await? {
                            let existing_id = artifact_id_for_bytes(&existing);
                            if existing_id != id {
                                return Err(Self::corruption(
                                    "artifact exists on disk but its contents do not match its id",
                                ));
                            }
                            return Ok(id);
                        }
                        return Err(Self::other(
                            "artifact appeared as existing, then disappeared during put",
                        ));
                    }
                    Err(create_err) => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return Err(Self::other(format!(
                            "failed to materialize artifact after hard-link fallback: {create_err}"
                        )));
                    }
                };

                if let Err(write_err) = dest.write_all(&bytes).await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(Self::other(format!(
                        "failed to write artifact after hard-link fallback: {write_err}"
                    )));
                }
                if let Err(sync_err) = dest.sync_all().await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(Self::other(format!(
                        "failed to sync artifact after hard-link fallback: {sync_err}"
                    )));
                }
                drop(dest);
                let _ = tokio::fs::remove_file(&temp_path).await;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(Self::other(format!("failed to materialize artifact: {e}")));
            }
        }

        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        let path = self.path_for(id);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Self::not_found("artifact not found"))
            }
            Err(e) => return Err(Self::other(format!("failed to read artifact: {e}"))),
        };

        let got_id = artifact_id_for_bytes(&bytes);
        if &got_id != id {
            return Err(Self::corruption("artifact contents hash did not match id"));
        }

        Ok(bytes)
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        let path = self.path_for(id);
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Self::other(format!("failed to stat artifact: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn get_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsArtifactStore::new(dir.path());

        let bytes = b"good".to_vec();
        let id = store
            .put(ArtifactKind::Other("test".to_string()), bytes)
            .await
            .unwrap();

        let path = store.path_for(&id);
        tokio::fs::write(path, b"bad").await.unwrap();

        match store.get(&id).await {
            Err(StorageError::Corruption(_)) => {}
            other => panic!("expected corruption, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_put_same_bytes_is_race_free() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FsArtifactStore::new(dir.path()));
        let bytes = b"shared-payload".to_vec();
        let kind = ArtifactKind::Other("race".to_string());
        let gate = Arc::new(tokio::sync::Barrier::new(32));

        let mut handles = Vec::new();
        for _ in 0..32 {
            let store = Arc::clone(&store);
            let payload = bytes.clone();
            let kind = kind.clone();
            let gate = Arc::clone(&gate);
            handles.push(tokio::spawn(async move {
                gate.wait().await;
                store.put(kind, payload).await
            }));
        }

        let mut expected = None;
        for handle in handles {
            let id = handle.await.unwrap().unwrap();
            if let Some(seen) = &expected {
                assert_eq!(&id, seen);
            } else {
                expected = Some(id);
            }
        }

        let id = expected.expect("at least one put result");
        let got = store.get(&id).await.unwrap();
        assert_eq!(got, bytes);
    }
}
