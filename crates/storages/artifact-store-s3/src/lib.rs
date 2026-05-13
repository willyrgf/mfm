#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! S3 and MinIO-backed `ArtifactStore`.
//!
//! Storage model:
//! - objects keyed by `ArtifactId` (SHA-256 hex)
//! - `put()` computes the id from bytes and uploads to `prefix/<id_prefix>/<id>`
//! - `get()` verifies the hash and returns `StorageError::Corruption` on mismatch
//!
//! # Examples
//!
//! ```no_run
//! use mfm_artifact_store_s3::S3ArtifactStore;
//!
//! let _store = S3ArtifactStore::from_env().expect("valid S3 configuration");
//! ```

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::{ArtifactId, ErrorCode};
use mfm_machine::stores::{ArtifactKind, ArtifactStore};

use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::{BucketConfiguration, Region};

/// S3-compatible immutable artifact store.
#[derive(Clone, Debug)]
pub struct S3ArtifactStore {
    bucket: Bucket,
    prefix: String,
}

impl S3ArtifactStore {
    /// Creates a store from an already configured bucket handle and object prefix.
    pub fn new(bucket: Bucket, prefix: impl Into<String>) -> Self {
        Self {
            bucket,
            prefix: prefix.into(),
        }
    }

    /// Builds a store from the standard `MFM_S3_*` environment variables.
    pub fn from_env() -> Result<Self, StorageError> {
        let endpoint = std::env::var("MFM_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string());
        let region_name =
            std::env::var("MFM_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let bucket_name = std::env::var("MFM_S3_BUCKET").unwrap_or_else(|_| "mfm-test".to_string());
        let prefix = std::env::var("MFM_S3_PREFIX").unwrap_or_else(|_| "mfm-artifacts".to_string());

        let credentials = Credentials::default().map_err(|e| {
            StorageError::Other(Self::info(
                "s3_invalid_auth",
                format!("invalid s3 credentials config: {e}"),
            ))
        })?;

        let region = Region::Custom {
            region: region_name,
            endpoint,
        };

        let bucket = Bucket::new(&bucket_name, region, credentials)
            .map_err(|e| StorageError::Other(Self::info("s3_invalid_config", e.to_string())))?;

        // MinIO compatibility: force path-style addressing.
        let bucket = *bucket.with_path_style();

        Ok(Self::new(bucket, prefix))
    }

    /// Ensures the configured bucket exists.
    ///
    /// This is a best-effort helper intended for tests and local development.
    pub async fn ensure_bucket_exists(&self) -> Result<(), StorageError> {
        let config = BucketConfiguration::default();
        let credentials = Credentials::default().map_err(|e| {
            StorageError::Other(Self::info(
                "s3_invalid_auth",
                format!("invalid s3 credentials config: {e}"),
            ))
        })?;

        match Bucket::create_with_path_style(
            &self.bucket.name,
            self.bucket.region.clone(),
            credentials,
            config,
        )
        .await
        {
            Ok(_) => Ok(()),
            Err(S3Error::HttpFailWithBody(409, _)) => Ok(()), // BucketAlreadyOwnedByYou / already exists
            Err(e) => Err(Self::map_s3_err("s3_create_bucket_failed", e)),
        }
    }

    fn object_path(&self, id: &ArtifactId) -> String {
        let prefix = self.prefix.trim_matches('/');
        let id_prefix = id.as_str().get(0..2).unwrap_or("xx");

        if prefix.is_empty() {
            format!("/{id_prefix}/{}", id.as_str())
        } else {
            format!("/{prefix}/{id_prefix}/{}", id.as_str())
        }
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

    fn map_s3_err(code: &'static str, e: S3Error) -> StorageError {
        match e {
            S3Error::HttpFailWithBody(404, _) => {
                StorageError::NotFound(Self::info(code, "artifact not found"))
            }
            other => StorageError::Other(Self::info(code, format!("s3 error: {other}"))),
        }
    }

    fn kind_to_meta(kind: &ArtifactKind) -> (&'static str, &'static str) {
        let v = match kind {
            ArtifactKind::Manifest => "manifest",
            ArtifactKind::ContextSnapshot => "context_snapshot",
            ArtifactKind::FactPayload => "fact_payload",
            ArtifactKind::SecretPayload => "secret_payload",
            ArtifactKind::Output => "output",
            ArtifactKind::Other(_) => "other",
        };
        ("mfm-kind", v)
    }

    async fn verify_existing_bytes(
        &self,
        id: &ArtifactId,
        expected: &[u8],
    ) -> Result<ArtifactId, StorageError> {
        let existing = self.get(id).await?;
        if existing != expected {
            return Err(StorageError::Corruption(Self::info(
                "artifact_corruption",
                "existing s3 artifact bytes do not match attempted write",
            )));
        }
        Ok(id.clone())
    }
}

#[async_trait]
impl ArtifactStore for S3ArtifactStore {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        let id = artifact_id_for_bytes(&bytes);
        let path = self.object_path(&id);
        let (k, v) = Self::kind_to_meta(&kind);

        if self.exists(&id).await? {
            return self.verify_existing_bytes(&id, &bytes).await;
        }

        let resp = self
            .bucket
            .put_object_builder(&path, &bytes)
            .with_metadata(k, v)
            .map_err(|e| StorageError::Other(Self::info("s3_put_failed", e.to_string())))?
            .with_header("If-None-Match", "*")
            .map_err(|e| StorageError::Other(Self::info("s3_put_failed", e.to_string())))?
            .execute()
            .await
            .map_err(|e| Self::map_s3_err("s3_put_failed", e))?;

        // rust-s3 does not fail on non-2xx by default; it returns a ResponseData with status_code.
        let status = resp.status_code();
        if matches!(status, 409 | 412) {
            return self.verify_existing_bytes(&id, &bytes).await;
        }
        if !(200..300).contains(&status) {
            return Err(StorageError::Other(Self::info(
                "s3_put_failed",
                format!("s3 put returned status {status}"),
            )));
        }

        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        let path = self.object_path(id);
        let data = self
            .bucket
            .get_object(&path)
            .await
            .map_err(|e| Self::map_s3_err("s3_get_failed", e))?;

        // rust-s3 does not fail on non-2xx by default; it returns a ResponseData with status_code.
        let status = data.status_code();
        if status == 404 {
            return Err(StorageError::NotFound(Self::info(
                "s3_get_failed",
                "artifact not found",
            )));
        }
        if !(200..300).contains(&status) {
            return Err(StorageError::Other(Self::info(
                "s3_get_failed",
                format!("s3 get returned status {status}"),
            )));
        }

        let bytes: Vec<u8> = data.into();

        let got_id = artifact_id_for_bytes(&bytes);
        if &got_id != id {
            return Err(StorageError::Corruption(Self::info(
                "artifact_corruption",
                "artifact contents hash did not match id",
            )));
        }

        Ok(bytes)
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        let path = self.object_path(id);
        self.bucket
            .object_exists(&path)
            .await
            .map_err(|e| Self::map_s3_err("s3_head_failed", e))
    }
}
