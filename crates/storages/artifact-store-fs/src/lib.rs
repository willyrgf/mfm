//! Filesystem `ArtifactStore` (fast lane, service-free).
//!
//! Storage model:
//! - content-addressed blobs keyed by `ArtifactId` (SHA-256 hex)
//! - `put()` computes the id from bytes
//! - `get()` verifies the hash and returns `StorageError::Corruption` on mismatch

use std::io;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::{ArtifactId, ErrorCode};
use mfm_machine::stores::{ArtifactKind, ArtifactStore};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug)]
pub struct FsArtifactStore {
    root: PathBuf,
}

impl FsArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, id: &ArtifactId) -> PathBuf {
        let prefix = id.0.get(0..2).unwrap_or("xx");
        self.root.join(prefix).join(&id.0)
    }

    fn info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
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

        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(id),
            Err(e) => return Err(Self::other(format!("failed to create artifact: {e}"))),
        };

        if let Err(e) = file.write_all(&bytes).await {
            // Best-effort cleanup; partial files are treated as corruption on read anyway.
            let _ = tokio::fs::remove_file(&path).await;
            return Err(Self::other(format!("failed to write artifact: {e}")));
        }
        if let Err(e) = file.sync_all().await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(Self::other(format!("failed to sync artifact: {e}")));
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
}
