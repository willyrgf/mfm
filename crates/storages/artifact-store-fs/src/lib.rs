#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Filesystem artifact stores for tests and local development.
//!
//! [`FsTypedArtifactStore`] is the certified typed local artifact store. It
//! stores immutable artifact bytes by digest and persists the evidence needed by
//! typed run commits: byte length, media type, schema id, semantic id, producer,
//! and artifact role.
//!
//! # Examples
//!
//! ```rust
//! use mfm_artifact_store_fs::FsTypedArtifactStore;
//!
//! let _store = FsTypedArtifactStore::new("/tmp/mfm-artifacts");
//! ```

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::{ArtifactRole, SeedCellRef};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, IdentityError, NodeId, SchemaId, SeedId,
    SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1::{ArtifactEvidenceRef, VerifiedRetentionProjectionSet};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result type for certified typed filesystem artifact operations.
pub type TypedArtifactResult<T> = std::result::Result<T, FsTypedArtifactError>;

/// Error returned by [`FsTypedArtifactStore`].
#[derive(Debug)]
pub enum FsTypedArtifactError {
    /// Artifact bytes or metadata were not present.
    NotFound {
        /// Missing artifact id.
        artifact_id: Box<ArtifactId>,
    },
    /// Persisted bytes or metadata are internally inconsistent.
    Corruption {
        /// Corrupt artifact id.
        artifact_id: Box<ArtifactId>,
        /// Stable diagnostic message without artifact bytes.
        message: String,
    },
    /// Supplied or persisted evidence does not match the artifact bytes or expected evidence.
    EvidenceMismatch {
        /// Artifact id being checked.
        artifact_id: Box<ArtifactId>,
        /// Evidence field that did not match.
        field: &'static str,
    },
    /// Garbage collection refused an artifact retained by a verified projection.
    RetainedArtifactRefused {
        /// Retained artifact id.
        artifact_id: Box<ArtifactId>,
    },
    /// Artifact evidence violates the typed artifact contract.
    InvalidEvidence {
        /// Stable diagnostic message without artifact bytes.
        message: String,
    },
    /// Identity or typed spec metadata failed validation.
    InvalidIdentity {
        /// Stable diagnostic message.
        message: String,
    },
    /// Filesystem operation failed.
    Io {
        /// Operation context.
        context: &'static str,
        /// Source I/O error.
        source: io::Error,
    },
}

impl fmt::Display for FsTypedArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { artifact_id } => write!(f, "typed artifact {artifact_id} not found"),
            Self::Corruption {
                artifact_id,
                message,
            } => write!(f, "typed artifact {artifact_id} corruption: {message}"),
            Self::EvidenceMismatch { artifact_id, field } => {
                write!(
                    f,
                    "typed artifact {artifact_id} evidence mismatch for {field}"
                )
            }
            Self::RetainedArtifactRefused { artifact_id } => {
                write!(f, "typed artifact {artifact_id} is retained")
            }
            Self::InvalidEvidence { message } => f.write_str(message),
            Self::InvalidIdentity { message } => f.write_str(message),
            Self::Io { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for FsTypedArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<IdentityError> for FsTypedArtifactError {
    fn from(error: IdentityError) -> Self {
        Self::InvalidIdentity {
            message: error.to_string(),
        }
    }
}

impl From<mfm_spec::SpecError> for FsTypedArtifactError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::InvalidIdentity {
            message: error.to_string(),
        }
    }
}

impl mfm_artifact_capabilities::ArtifactReadProvider for FsTypedArtifactStore {
    fn read_artifact<'a>(
        &'a self,
        request: &'a mfm_artifact_capabilities::ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::ArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .get_artifact_by_id(request.artifact_id())
                .await
                .map_err(redact_artifact_capability_error)?;
            let evidence = mfm_artifact_capabilities::ArtifactEvidenceRef::from(evidence);
            mfm_artifact_capabilities::VerifiedArtifactBytes::new(bytes, evidence, request)
        })
    }
}

/// Evidence fields supplied when storing typed artifact bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedArtifactDescriptor {
    /// Artifact media type.
    pub media_type: MediaType,
    /// Artifact schema id, when schema-bearing.
    pub schema_id: Option<SchemaId>,
    /// Artifact semantic type id, when value-bearing.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Producer node id, when produced by a state node.
    pub producer_node_id: Option<NodeId>,
    /// Producer seed id, when produced by a launch seed.
    pub producer_seed_id: Option<SeedId>,
    /// Artifact role.
    pub artifact_role: ArtifactRole,
}

/// Certified local filesystem artifact store for typed run artifacts.
#[derive(Clone, Debug)]
pub struct FsTypedArtifactStore {
    root: PathBuf,
}

impl FsTypedArtifactStore {
    /// Creates a typed artifact store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Stores artifact bytes and returns persisted typed evidence.
    pub async fn put_artifact(
        &self,
        bytes: Vec<u8>,
        descriptor: TypedArtifactDescriptor,
    ) -> TypedArtifactResult<ArtifactEvidenceRef> {
        let digest = content_digest_for_bytes(&bytes);
        let artifact_id = ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest());
        let byte_len = bytes.len() as u64;
        let evidence = ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len,
            media_type: descriptor.media_type,
            schema_id: descriptor.schema_id,
            semantic_type_id: descriptor.semantic_type_id,
            producer_node_id: descriptor.producer_node_id,
            producer_seed_id: descriptor.producer_seed_id,
            artifact_role: descriptor.artifact_role,
        };
        self.put_verified_artifact(bytes, evidence).await
    }

    /// Stores artifact bytes after verifying they match caller-supplied evidence.
    pub async fn put_verified_artifact(
        &self,
        bytes: Vec<u8>,
        evidence: ArtifactEvidenceRef,
    ) -> TypedArtifactResult<ArtifactEvidenceRef> {
        validate_evidence_shape(&evidence)?;
        verify_evidence_matches_bytes(&bytes, &evidence)?;

        let blob_path = self.blob_path_for(&evidence.artifact_id);
        write_blob_if_absent(&blob_path, &bytes, &evidence.artifact_id).await?;

        let metadata_path = self.metadata_path_for(&evidence.artifact_id);
        write_metadata_if_absent(&metadata_path, &evidence).await?;
        Ok(evidence)
    }

    /// Loads artifact bytes and verifies bytes plus metadata against the supplied evidence.
    pub async fn get_artifact(
        &self,
        evidence: &ArtifactEvidenceRef,
    ) -> TypedArtifactResult<Vec<u8>> {
        validate_evidence_shape(evidence)?;
        let stored = self.load_metadata(&evidence.artifact_id).await?;
        compare_evidence(&stored, evidence)?;

        let blob_path = self.blob_path_for(&evidence.artifact_id);
        let bytes = match tokio::fs::read(&blob_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(FsTypedArtifactError::NotFound {
                    artifact_id: Box::new(evidence.artifact_id.clone()),
                });
            }
            Err(source) => {
                return Err(FsTypedArtifactError::Io {
                    context: "failed to read typed artifact bytes",
                    source,
                });
            }
        };
        verify_evidence_matches_bytes(&bytes, evidence)?;
        Ok(bytes)
    }

    /// Loads typed artifact bytes by id and returns the persisted evidence.
    ///
    /// This is intended for public-output rendering and replay assembly paths that start from a
    /// store-owned projected artifact id. The method verifies metadata and bytes before returning.
    pub async fn get_artifact_by_id(
        &self,
        artifact_id: &ArtifactId,
    ) -> TypedArtifactResult<(Vec<u8>, ArtifactEvidenceRef)> {
        let evidence = self.load_metadata(artifact_id).await?;
        let bytes = self.get_artifact(&evidence).await?;
        Ok((bytes, evidence))
    }

    /// Returns true only when bytes and metadata both match the supplied evidence.
    pub async fn has_artifact(&self, evidence: &ArtifactEvidenceRef) -> TypedArtifactResult<bool> {
        match self.get_artifact(evidence).await {
            Ok(_) => Ok(true),
            Err(FsTypedArtifactError::NotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Loads mandatory seed material and verifies seed producer evidence.
    pub async fn require_seed_material(&self, seed: &SeedCellRef) -> TypedArtifactResult<Vec<u8>> {
        let evidence = seed_artifact_evidence(seed)?;
        self.get_artifact(&evidence).await
    }

    /// Removes an artifact only when a complete verified retention set does not protect it.
    pub async fn remove_unretained_artifact(
        &self,
        evidence: &ArtifactEvidenceRef,
        retention: &VerifiedRetentionProjectionSet,
    ) -> TypedArtifactResult<()> {
        validate_evidence_shape(evidence)?;
        if retention.retains_artifact(evidence) {
            return Err(FsTypedArtifactError::RetainedArtifactRefused {
                artifact_id: Box::new(evidence.artifact_id.clone()),
            });
        }
        self.get_artifact(evidence).await?;
        let metadata_path = self.metadata_path_for(&evidence.artifact_id);
        let blob_path = self.blob_path_for(&evidence.artifact_id);
        tokio::fs::remove_file(&metadata_path)
            .await
            .map_err(|source| FsTypedArtifactError::Io {
                context: "failed to remove typed artifact metadata",
                source,
            })?;
        tokio::fs::remove_file(&blob_path)
            .await
            .map_err(|source| FsTypedArtifactError::Io {
                context: "failed to remove typed artifact bytes",
                source,
            })?;
        Ok(())
    }

    fn typed_root(&self) -> PathBuf {
        self.root.join("typed")
    }

    fn blob_path_for(&self, artifact_id: &ArtifactId) -> PathBuf {
        let digest = artifact_id.digest().to_string();
        self.typed_root()
            .join("blobs")
            .join(&digest[0..2])
            .join(artifact_id.as_str())
    }

    fn metadata_path_for(&self, artifact_id: &ArtifactId) -> PathBuf {
        let digest = artifact_id.digest().to_string();
        self.typed_root()
            .join("metadata")
            .join(&digest[0..2])
            .join(format!("{}.json", artifact_id.as_str()))
    }

    async fn load_metadata(
        &self,
        artifact_id: &ArtifactId,
    ) -> TypedArtifactResult<ArtifactEvidenceRef> {
        let path = self.metadata_path_for(artifact_id);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(FsTypedArtifactError::NotFound {
                    artifact_id: Box::new(artifact_id.clone()),
                });
            }
            Err(source) => {
                return Err(FsTypedArtifactError::Io {
                    context: "failed to read typed artifact metadata",
                    source,
                });
            }
        };
        let json: Value =
            serde_json::from_slice(&bytes).map_err(|error| FsTypedArtifactError::Corruption {
                artifact_id: Box::new(artifact_id.clone()),
                message: format!("invalid typed artifact metadata JSON: {error}"),
            })?;
        let evidence = evidence_from_json(&json)?;
        if evidence.artifact_id != *artifact_id {
            return Err(FsTypedArtifactError::Corruption {
                artifact_id: Box::new(artifact_id.clone()),
                message: "metadata artifact id does not match metadata path".to_owned(),
            });
        }
        validate_evidence_shape(&evidence)?;
        Ok(evidence)
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

fn content_digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn validate_evidence_shape(evidence: &ArtifactEvidenceRef) -> TypedArtifactResult<()> {
    if evidence.digest.algorithm() != DigestAlgorithm::Sha256JcsV1
        || evidence.artifact_id.algorithm() != DigestAlgorithm::Sha256JcsV1
    {
        return Err(FsTypedArtifactError::InvalidEvidence {
            message: "typed artifacts must use sha256-jcs-v1 digest identities".to_owned(),
        });
    }
    if evidence.artifact_id.digest() != evidence.digest.digest() {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(evidence.artifact_id.clone()),
            field: "artifact_id",
        });
    }
    if evidence.producer_node_id.is_some() && evidence.producer_seed_id.is_some() {
        return Err(FsTypedArtifactError::InvalidEvidence {
            message: "typed artifact evidence cannot have both node and seed producers".to_owned(),
        });
    }
    match evidence.artifact_role {
        ArtifactRole::SeedInput => {
            if evidence.producer_seed_id.is_none() {
                return Err(FsTypedArtifactError::InvalidEvidence {
                    message: "seed input artifacts require producer_seed_id".to_owned(),
                });
            }
            if evidence.producer_node_id.is_some() {
                return Err(FsTypedArtifactError::InvalidEvidence {
                    message: "seed input artifacts cannot have producer_node_id".to_owned(),
                });
            }
        }
        ArtifactRole::TypedExecutionSpec
        | ArtifactRole::TypedSpecCertificate
        | ArtifactRole::TypedConfig
        | ArtifactRole::RedactedDiagnostic
        | ArtifactRole::RetentionManifest => {}
        _ => {
            if evidence.producer_node_id.is_none() {
                return Err(FsTypedArtifactError::InvalidEvidence {
                    message: format!(
                        "{} artifacts require producer_node_id",
                        artifact_role_str(evidence.artifact_role)
                    ),
                });
            }
        }
    }
    if evidence.artifact_role != ArtifactRole::SeedInput && evidence.producer_seed_id.is_some() {
        return Err(FsTypedArtifactError::InvalidEvidence {
            message: "only seed input artifacts may carry producer_seed_id".to_owned(),
        });
    }
    Ok(())
}

fn verify_evidence_matches_bytes(
    bytes: &[u8],
    evidence: &ArtifactEvidenceRef,
) -> TypedArtifactResult<()> {
    let digest = content_digest_for_bytes(bytes);
    if evidence.digest != digest {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(evidence.artifact_id.clone()),
            field: "content_digest",
        });
    }
    let artifact_id = ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest());
    if evidence.artifact_id != artifact_id {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(evidence.artifact_id.clone()),
            field: "artifact_id",
        });
    }
    if evidence.byte_len != bytes.len() as u64 {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(evidence.artifact_id.clone()),
            field: "byte_len",
        });
    }
    Ok(())
}

fn compare_evidence(
    stored: &ArtifactEvidenceRef,
    expected: &ArtifactEvidenceRef,
) -> TypedArtifactResult<()> {
    compare_evidence_field(stored, expected, "artifact_id", |evidence| {
        evidence.artifact_id.as_str().to_owned()
    })?;
    compare_evidence_field(stored, expected, "content_digest", |evidence| {
        evidence.digest.as_str().to_owned()
    })?;
    compare_evidence_field(stored, expected, "byte_len", |evidence| evidence.byte_len)?;
    compare_evidence_field(stored, expected, "media_type", |evidence| {
        evidence.media_type.as_str().to_owned()
    })?;
    compare_evidence_field(stored, expected, "schema_id", |evidence| {
        evidence
            .schema_id
            .as_ref()
            .map(|schema_id| schema_id.as_str().to_owned())
    })?;
    compare_evidence_field(stored, expected, "semantic_type_id", |evidence| {
        evidence
            .semantic_type_id
            .as_ref()
            .map(|semantic_type_id| semantic_type_id.as_str().to_owned())
    })?;
    compare_evidence_field(stored, expected, "artifact_role", |evidence| {
        artifact_role_str(evidence.artifact_role).to_owned()
    })?;
    compare_evidence_field(stored, expected, "producer_node_id", |evidence| {
        evidence
            .producer_node_id
            .as_ref()
            .map(|node_id| node_id.as_str().to_owned())
    })?;
    compare_evidence_field(stored, expected, "producer_seed_id", |evidence| {
        evidence
            .producer_seed_id
            .as_ref()
            .map(|seed_id| seed_id.as_str().to_owned())
    })?;
    Ok(())
}

fn compare_evidence_field<T: PartialEq>(
    stored: &ArtifactEvidenceRef,
    expected: &ArtifactEvidenceRef,
    field: &'static str,
    accessor: impl Fn(&ArtifactEvidenceRef) -> T,
) -> TypedArtifactResult<()> {
    if accessor(stored) == accessor(expected) {
        return Ok(());
    }
    Err(FsTypedArtifactError::EvidenceMismatch {
        artifact_id: Box::new(expected.artifact_id.clone()),
        field,
    })
}

fn redact_artifact_capability_error(
    error: FsTypedArtifactError,
) -> mfm_artifact_capabilities::ArtifactReadError {
    match error {
        FsTypedArtifactError::NotFound { artifact_id } => {
            mfm_artifact_capabilities::ArtifactReadError::NotFound { artifact_id }
        }
        FsTypedArtifactError::EvidenceMismatch { artifact_id, field } => {
            mfm_artifact_capabilities::ArtifactReadError::EvidenceMismatch { artifact_id, field }
        }
        error @ (FsTypedArtifactError::InvalidEvidence { .. }
        | FsTypedArtifactError::InvalidIdentity { .. }
        | FsTypedArtifactError::Corruption { .. }
        | FsTypedArtifactError::RetainedArtifactRefused { .. }
        | FsTypedArtifactError::Io { .. }) => {
            mfm_artifact_capabilities::ArtifactReadError::redacted_backend_failure(error)
        }
    }
}

fn seed_artifact_evidence(seed: &SeedCellRef) -> TypedArtifactResult<ArtifactEvidenceRef> {
    let artifact = &seed.seed_artifact;
    if artifact.role != ArtifactRole::SeedInput {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(artifact.artifact_id.clone()),
            field: "artifact_role",
        });
    }
    if artifact.content_digest != seed.digest {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(artifact.artifact_id.clone()),
            field: "content_digest",
        });
    }
    if artifact.schema_id != seed.schema_id {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(artifact.artifact_id.clone()),
            field: "schema_id",
        });
    }
    if artifact.semantic_type_id.as_ref() != Some(&seed.semantic_type_id) {
        return Err(FsTypedArtifactError::EvidenceMismatch {
            artifact_id: Box::new(artifact.artifact_id.clone()),
            field: "semantic_type_id",
        });
    }
    Ok(ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
        schema_id: Some(artifact.schema_id.clone()),
        semantic_type_id: Some(seed.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: ArtifactRole::SeedInput,
    })
}

async fn write_blob_if_absent(
    path: &Path,
    bytes: &[u8],
    artifact_id: &ArtifactId,
) -> TypedArtifactResult<()> {
    match tokio::fs::read(path).await {
        Ok(existing) => {
            let expected =
                ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, *artifact_id.digest());
            let actual = content_digest_for_bytes(&existing);
            if actual != expected {
                return Err(FsTypedArtifactError::Corruption {
                    artifact_id: Box::new(artifact_id.clone()),
                    message: "artifact bytes do not match artifact id".to_owned(),
                });
            }
            if existing != bytes {
                return Err(FsTypedArtifactError::Corruption {
                    artifact_id: Box::new(artifact_id.clone()),
                    message: "artifact id collision with different bytes".to_owned(),
                });
            }
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(FsTypedArtifactError::Io {
                context: "failed to read existing typed artifact bytes",
                source,
            });
        }
    }

    write_new_file(path, bytes).await?;
    let installed = tokio::fs::read(path)
        .await
        .map_err(|source| FsTypedArtifactError::Io {
            context: "failed to verify installed typed artifact bytes",
            source,
        })?;
    if installed != bytes {
        return Err(FsTypedArtifactError::Corruption {
            artifact_id: Box::new(artifact_id.clone()),
            message: "typed artifact bytes changed during concurrent install".to_owned(),
        });
    }
    Ok(())
}

async fn write_metadata_if_absent(
    path: &Path,
    evidence: &ArtifactEvidenceRef,
) -> TypedArtifactResult<()> {
    match tokio::fs::read(path).await {
        Ok(existing) => {
            let json: Value = serde_json::from_slice(&existing).map_err(|error| {
                FsTypedArtifactError::Corruption {
                    artifact_id: Box::new(evidence.artifact_id.clone()),
                    message: format!("invalid typed artifact metadata JSON: {error}"),
                }
            })?;
            let stored = evidence_from_json(&json)?;
            compare_evidence(&stored, evidence)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let bytes = serde_json::to_vec(&evidence_json(evidence)).map_err(|error| {
                FsTypedArtifactError::Corruption {
                    artifact_id: Box::new(evidence.artifact_id.clone()),
                    message: format!("failed to encode typed artifact metadata: {error}"),
                }
            })?;
            write_new_file(path, &bytes).await?;
            let installed =
                tokio::fs::read(path)
                    .await
                    .map_err(|source| FsTypedArtifactError::Io {
                        context: "failed to verify installed typed artifact metadata",
                        source,
                    })?;
            let json: Value = serde_json::from_slice(&installed).map_err(|error| {
                FsTypedArtifactError::Corruption {
                    artifact_id: Box::new(evidence.artifact_id.clone()),
                    message: format!("invalid typed artifact metadata JSON: {error}"),
                }
            })?;
            let stored = evidence_from_json(&json)?;
            compare_evidence(&stored, evidence)
        }
        Err(source) => Err(FsTypedArtifactError::Io {
            context: "failed to read existing typed artifact metadata",
            source,
        }),
    }
}

async fn write_new_file(path: &Path, bytes: &[u8]) -> TypedArtifactResult<()> {
    let Some(parent) = path.parent() else {
        return Err(FsTypedArtifactError::InvalidEvidence {
            message: "typed artifact path had no parent directory".to_owned(),
        });
    };
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| FsTypedArtifactError::Io {
            context: "failed to create typed artifact directory",
            source,
        })?;
    let temp_path = temp_path_for(path);
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .await
    {
        Ok(file) => file,
        Err(source) => {
            return Err(FsTypedArtifactError::Io {
                context: "failed to create typed artifact temp file",
                source,
            });
        }
    };
    if let Err(source) = file.write_all(bytes).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(FsTypedArtifactError::Io {
            context: "failed to write typed artifact temp file",
            source,
        });
    }
    if let Err(source) = file.sync_all().await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(FsTypedArtifactError::Io {
            context: "failed to sync typed artifact temp file",
            source,
        });
    }
    drop(file);

    match tokio::fs::hard_link(&temp_path, path).await {
        Ok(()) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            Ok(())
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            Ok(())
        }
        Err(source) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            Err(FsTypedArtifactError::Io {
                context: "failed to install typed artifact file",
                source,
            })
        }
    }
}

fn evidence_json(evidence: &ArtifactEvidenceRef) -> Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "content_digest": evidence.digest.as_str(),
        "byte_len": evidence.byte_len,
        "media_type": evidence.media_type.as_str(),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
        "producer_node_id": evidence.producer_node_id.as_ref().map(NodeId::as_str),
        "producer_seed_id": evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        "artifact_role": artifact_role_str(evidence.artifact_role),
    })
}

fn evidence_from_json(json: &Value) -> TypedArtifactResult<ArtifactEvidenceRef> {
    Ok(ArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        digest: parse_identity(required_str(json, "content_digest")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)?,
        schema_id: optional_identity(json, "schema_id")?,
        semantic_type_id: optional_identity(json, "semantic_type_id")?,
        producer_node_id: optional_identity(json, "producer_node_id")?,
        producer_seed_id: optional_identity(json, "producer_seed_id")?,
        artifact_role: parse_artifact_role(required_str(json, "artifact_role")?)?,
    })
}

fn required_str<'a>(json: &'a Value, field: &'static str) -> TypedArtifactResult<&'a str> {
    json.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| FsTypedArtifactError::InvalidEvidence {
            message: format!("typed artifact metadata missing string field {field}"),
        })
}

fn required_u64(json: &Value, field: &'static str) -> TypedArtifactResult<u64> {
    json.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| FsTypedArtifactError::InvalidEvidence {
            message: format!("typed artifact metadata missing u64 field {field}"),
        })
}

fn optional_identity<T>(json: &Value, field: &'static str) -> TypedArtifactResult<Option<T>>
where
    T: std::str::FromStr<Err = IdentityError>,
{
    match json.get(field) {
        Some(Value::String(value)) => Ok(Some(parse_identity(value)?)),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(FsTypedArtifactError::InvalidEvidence {
            message: format!("typed artifact metadata field {field} must be a string or null"),
        }),
    }
}

fn parse_identity<T>(value: &str) -> TypedArtifactResult<T>
where
    T: std::str::FromStr<Err = IdentityError>,
{
    value.parse().map_err(FsTypedArtifactError::from)
}

fn artifact_role_str(role: ArtifactRole) -> &'static str {
    match role {
        ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
        ArtifactRole::TypedSpecCertificate => "typed_spec_certificate",
        ArtifactRole::TypedConfig => "typed_config",
        ArtifactRole::SeedInput => "seed_input",
        ArtifactRole::StateOutput => "state_output",
        ArtifactRole::FactResponse => "fact_response",
        ArtifactRole::SideEffectIntent => "side_effect_intent",
        ArtifactRole::PreparedInvocation => "prepared_invocation",
        ArtifactRole::NotSubmittedProof => "not_submitted_proof",
        ArtifactRole::Submission => "submission",
        ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
        ArtifactRole::Receipt => "receipt",
        ArtifactRole::Confirmation => "confirmation",
        ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
        ArtifactRole::PublicOutput => "public_output",
        ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

fn parse_artifact_role(value: &str) -> TypedArtifactResult<ArtifactRole> {
    match value {
        "typed_execution_spec" => Ok(ArtifactRole::TypedExecutionSpec),
        "typed_spec_certificate" => Ok(ArtifactRole::TypedSpecCertificate),
        "typed_config" => Ok(ArtifactRole::TypedConfig),
        "seed_input" => Ok(ArtifactRole::SeedInput),
        "state_output" => Ok(ArtifactRole::StateOutput),
        "fact_response" => Ok(ArtifactRole::FactResponse),
        "side_effect_intent" => Ok(ArtifactRole::SideEffectIntent),
        "prepared_invocation" => Ok(ArtifactRole::PreparedInvocation),
        "not_submitted_proof" => Ok(ArtifactRole::NotSubmittedProof),
        "submission" => Ok(ArtifactRole::Submission),
        "submission_unknown_evidence" => Ok(ArtifactRole::SubmissionUnknownEvidence),
        "receipt" => Ok(ArtifactRole::Receipt),
        "confirmation" => Ok(ArtifactRole::Confirmation),
        "ambiguity_evidence" => Ok(ArtifactRole::AmbiguityEvidence),
        "public_output" => Ok(ArtifactRole::PublicOutput),
        "redacted_diagnostic" => Ok(ArtifactRole::RedactedDiagnostic),
        "retention_manifest" => Ok(ArtifactRole::RetentionManifest),
        _ => Err(FsTypedArtifactError::InvalidEvidence {
            message: format!("unknown artifact role {value}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use mfm_artifact_capabilities::{ArtifactReadProvider, ArtifactReadRequest};
    use mfm_ids::{DigestAlgorithm, SemanticTypeId};

    use super::*;

    fn schema_id(name: &str) -> SchemaId {
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(name.as_bytes()),
        )
        .expect("schema id")
    }

    fn semantic_id(name: &str) -> SemanticTypeId {
        SemanticTypeId::new(
            "mfm.test",
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(name.as_bytes()),
        )
        .expect("semantic id")
    }

    fn node_id(name: &str) -> NodeId {
        NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(name.as_bytes()),
        )
    }

    fn descriptor() -> TypedArtifactDescriptor {
        TypedArtifactDescriptor {
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.artifact")),
            semantic_type_id: Some(semantic_id("artifact")),
            producer_node_id: Some(node_id("producer")),
            producer_seed_id: None,
            artifact_role: ArtifactRole::StateOutput,
        }
    }

    #[tokio::test]
    async fn filesystem_store_implements_artifact_read_provider() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = FsTypedArtifactStore::new(temp.path());
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = store
            .put_artifact(bytes.clone(), descriptor())
            .await
            .expect("put artifact");
        let request = ArtifactReadRequest::from_replay_authorized_evidence(evidence.clone().into());

        let verified = store
            .read_artifact(&request)
            .await
            .expect("read artifact through capability");

        assert_eq!(verified.bytes(), bytes.as_slice());
        assert_eq!(verified.evidence().artifact_id, evidence.artifact_id);
    }

    #[tokio::test]
    async fn filesystem_provider_maps_missing_artifact_to_redacted_error() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = FsTypedArtifactStore::new(temp.path());
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(&bytes),
            ),
            digest: content_digest_for_bytes(&bytes),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.artifact")),
            semantic_type_id: Some(semantic_id("artifact")),
            producer_node_id: Some(node_id("producer")),
            producer_seed_id: None,
            artifact_role: ArtifactRole::StateOutput,
        };
        let request = ArtifactReadRequest::from_replay_authorized_evidence(evidence.into());

        let err = store
            .read_artifact(&request)
            .await
            .expect_err("missing artifact");
        let rendered = err.to_string();

        assert!(matches!(
            err,
            mfm_artifact_capabilities::ArtifactReadError::NotFound { .. }
        ));
        assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn filesystem_provider_redacts_backend_error_sources() {
        let source_path = "/tmp/secret/mfm-key.json";
        let err = redact_artifact_capability_error(FsTypedArtifactError::Io {
            context: "failed to read typed artifact bytes",
            source: std::io::Error::new(std::io::ErrorKind::Other, source_path),
        });
        let rendered = err.to_string();

        assert!(matches!(
            err,
            mfm_artifact_capabilities::ArtifactReadError::Backend {
                reason: mfm_artifact_capabilities::ArtifactReadBackendError::Failed
            }
        ));
        assert!(!rendered.contains(source_path));
        assert!(!rendered.contains("/tmp/secret"));
    }
}
