#![warn(missing_docs)]
//! Artifact read capability contracts.
//!
//! This crate defines the state/adapter-facing contract for reading verified typed artifact
//! bytes. It owns artifact-read requests, evidence expectations, verified response bytes, and
//! redaction-safe errors. Concrete stores and object backends implement [`ArtifactReadProvider`]
//! elsewhere.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec, ReadExternalRole};
use mfm_events::v1::ArtifactRole;
use mfm_facts::InternalFactRef;
use mfm_ids::{
    ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest, DigestAlgorithm, NodeId,
    SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::{self as spec, MediaType};
use mfm_store::v1 as store;
use serde::de::DeserializeOwned;

/// Result type for artifact-read capability contracts.
pub type Result<T> = std::result::Result<T, ArtifactReadError>;

/// Boxed future returned by artifact-read providers.
pub type ArtifactReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VerifiedArtifactBytes>> + Send + 'a>>;

/// Stable capability descriptor for verified artifact reads.
pub struct ArtifactReadCapability;

impl CapabilitySpec for ArtifactReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.artifact",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.artifact.capability:read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.artifact.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.artifact.read"
    }
}

/// Provider interface for verified artifact reads.
pub trait ArtifactReadProvider: Send + Sync {
    /// Reads artifact bytes and verifies them against the request evidence.
    fn read_artifact<'a>(&'a self, request: &'a ArtifactReadRequest) -> ArtifactReadFuture<'a>;
}

/// Full typed artifact evidence returned by an artifact-read provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactEvidenceRef {
    /// Artifact id.
    pub artifact_id: ArtifactId,
    /// Artifact content digest.
    pub digest: ContentDigest,
    /// Artifact byte length.
    pub byte_len: u64,
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

impl ArtifactEvidenceRef {
    /// Builds artifact evidence from the store-owned evidence shape.
    pub fn from_store(evidence: store::ArtifactEvidenceRef) -> Self {
        Self {
            artifact_id: evidence.artifact_id,
            digest: evidence.digest,
            byte_len: evidence.byte_len,
            media_type: evidence.media_type,
            schema_id: evidence.schema_id,
            semantic_type_id: evidence.semantic_type_id,
            producer_node_id: evidence.producer_node_id,
            producer_seed_id: evidence.producer_seed_id,
            artifact_role: evidence.artifact_role,
        }
    }

    /// Converts this contract evidence into the store-owned evidence shape.
    pub fn into_store(self) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id: self.artifact_id,
            digest: self.digest,
            byte_len: self.byte_len,
            media_type: self.media_type,
            schema_id: self.schema_id,
            semantic_type_id: self.semantic_type_id,
            producer_node_id: self.producer_node_id,
            producer_seed_id: self.producer_seed_id,
            artifact_role: self.artifact_role,
        }
    }
}

impl From<store::ArtifactEvidenceRef> for ArtifactEvidenceRef {
    fn from(evidence: store::ArtifactEvidenceRef) -> Self {
        Self::from_store(evidence)
    }
}

impl From<ArtifactEvidenceRef> for store::ArtifactEvidenceRef {
    fn from(evidence: ArtifactEvidenceRef) -> Self {
        evidence.into_store()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactEvidenceExpectation {
    artifact_id: ArtifactId,
    digest: Option<ContentDigest>,
    byte_len: Option<u64>,
    media_type: Option<MediaType>,
    schema_id: OptionalEvidence<SchemaId>,
    semantic_type_id: OptionalEvidence<SemanticTypeId>,
    producer_node_id: OptionalEvidence<NodeId>,
    producer_seed_id: OptionalEvidence<SeedId>,
    artifact_role: Option<ArtifactRole>,
}

impl ArtifactEvidenceExpectation {
    fn exact(evidence: &ArtifactEvidenceRef) -> Self {
        Self {
            artifact_id: evidence.artifact_id.clone(),
            digest: Some(evidence.digest.clone()),
            byte_len: Some(evidence.byte_len),
            media_type: Some(evidence.media_type.clone()),
            schema_id: optional_exact(evidence.schema_id.clone()),
            semantic_type_id: optional_exact(evidence.semantic_type_id.clone()),
            producer_node_id: optional_exact(evidence.producer_node_id.clone()),
            producer_seed_id: optional_exact(evidence.producer_seed_id.clone()),
            artifact_role: Some(evidence.artifact_role),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OptionalEvidence<T> {
    Any,
    Absent,
    Present(T),
}

impl<T> OptionalEvidence<T> {
    fn matches(&self, actual: &Option<T>) -> bool
    where
        T: PartialEq,
    {
        match self {
            Self::Any => true,
            Self::Absent => actual.is_none(),
            Self::Present(expected) => actual.as_ref() == Some(expected),
        }
    }
}

/// Typed artifact read request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadRequest {
    expectation: ArtifactEvidenceExpectation,
}

impl ArtifactReadRequest {
    fn from_expectation(expectation: ArtifactEvidenceExpectation) -> Self {
        Self { expectation }
    }

    /// Creates a request from full replay-authorized artifact evidence.
    pub fn from_replay_authorized_evidence(evidence: ArtifactEvidenceRef) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation::exact(&evidence))
    }

    /// Creates a request for a fact response artifact pinned by an internal fact ref.
    pub fn from_internal_fact_response_ref(fact_ref: &InternalFactRef) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id: fact_ref.artifact_id().clone(),
            digest: Some(fact_ref.response_hash().clone()),
            byte_len: None,
            media_type: None,
            schema_id: OptionalEvidence::Present(fact_ref.response_schema_id().clone()),
            semantic_type_id: OptionalEvidence::Any,
            producer_node_id: OptionalEvidence::Any,
            producer_seed_id: OptionalEvidence::Any,
            artifact_role: Some(ArtifactRole::FactResponse),
        })
    }

    /// Creates a request for a certified config artifact reference.
    pub fn from_certified_config_ref(config_ref: &spec::ConfigRef) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id: config_ref.artifact_id.clone(),
            digest: Some(config_ref.digest.clone()),
            byte_len: Some(config_ref.byte_len),
            media_type: Some(config_ref.media_type.clone()),
            schema_id: OptionalEvidence::Present(config_ref.schema_id.clone()),
            semantic_type_id: OptionalEvidence::Absent,
            producer_node_id: OptionalEvidence::Absent,
            producer_seed_id: OptionalEvidence::Absent,
            artifact_role: Some(ArtifactRole::TypedConfig),
        })
    }

    /// Creates a request from a materialized state-output cell projection.
    pub fn from_materialized_cell(terminal: &store::CellTerminalProjection) -> Result<Self> {
        match terminal {
            store::CellTerminalProjection::Produced {
                node_id,
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                ..
            } => Ok(Self::from_expectation(ArtifactEvidenceExpectation {
                artifact_id: artifact_id.clone(),
                digest: Some(content_digest.clone()),
                byte_len: None,
                media_type: None,
                schema_id: OptionalEvidence::Present(schema_id.clone()),
                semantic_type_id: OptionalEvidence::Present(semantic_type_id.clone()),
                producer_node_id: OptionalEvidence::Present(node_id.clone()),
                producer_seed_id: OptionalEvidence::Absent,
                artifact_role: Some(ArtifactRole::StateOutput),
            })),
            store::CellTerminalProjection::Skipped { .. } => {
                Err(ArtifactReadError::InvalidRequest {
                    reason: ArtifactReadInvalidRequest::SkippedCell,
                })
            }
        }
    }

    /// Creates a request from runtime materialized seed-cell evidence.
    pub fn from_materialized_seed_cell(
        artifact_id: ArtifactId,
        digest: ContentDigest,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        seed_id: SeedId,
    ) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id,
            digest: Some(digest),
            byte_len: None,
            media_type: None,
            schema_id: OptionalEvidence::Present(schema_id),
            semantic_type_id: OptionalEvidence::Present(semantic_type_id),
            producer_node_id: OptionalEvidence::Absent,
            producer_seed_id: OptionalEvidence::Present(seed_id),
            artifact_role: Some(ArtifactRole::SeedInput),
        })
    }

    /// Creates a request from runtime materialized produced-cell evidence.
    pub fn from_materialized_produced_cell(
        artifact_id: ArtifactId,
        digest: ContentDigest,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        producer_node_id: NodeId,
    ) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id,
            digest: Some(digest),
            byte_len: None,
            media_type: None,
            schema_id: OptionalEvidence::Present(schema_id),
            semantic_type_id: OptionalEvidence::Present(semantic_type_id),
            producer_node_id: OptionalEvidence::Present(producer_node_id),
            producer_seed_id: OptionalEvidence::Absent,
            artifact_role: Some(ArtifactRole::StateOutput),
        })
    }

    /// Creates a request from side-effect artifact projection evidence.
    pub fn from_side_effect_projection(
        artifact: &store::SideEffectArtifactProjection,
        artifact_role: ArtifactRole,
        producer_node_id: NodeId,
    ) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id: artifact.artifact_id.clone(),
            digest: Some(artifact.content_digest.clone()),
            byte_len: None,
            media_type: None,
            schema_id: match artifact.schema_id.clone() {
                Some(schema_id) => OptionalEvidence::Present(schema_id),
                None => OptionalEvidence::Any,
            },
            semantic_type_id: OptionalEvidence::Any,
            producer_node_id: OptionalEvidence::Present(producer_node_id),
            producer_seed_id: OptionalEvidence::Absent,
            artifact_role: Some(artifact_role),
        })
    }

    /// Returns the artifact id to read.
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.expectation.artifact_id
    }

    /// Returns the expected content digest, when the request constrains it.
    pub fn digest(&self) -> Option<&ContentDigest> {
        self.expectation.digest.as_ref()
    }

    /// Returns the expected byte length, when the request constrains it.
    pub fn byte_len(&self) -> Option<u64> {
        self.expectation.byte_len
    }

    /// Returns the expected media type, when the request constrains it.
    pub fn media_type(&self) -> Option<&MediaType> {
        self.expectation.media_type.as_ref()
    }

    /// Returns the expected schema id, when the request constrains it to a concrete value.
    pub fn schema_id(&self) -> Option<&SchemaId> {
        optional_present_ref(&self.expectation.schema_id)
    }

    /// Returns the expected semantic type id, when the request constrains it to a concrete value.
    pub fn semantic_type_id(&self) -> Option<&SemanticTypeId> {
        optional_present_ref(&self.expectation.semantic_type_id)
    }

    /// Returns the expected producer node id, when the request constrains it to a concrete value.
    pub fn producer_node_id(&self) -> Option<&NodeId> {
        optional_present_ref(&self.expectation.producer_node_id)
    }

    /// Returns the expected producer seed id, when the request constrains it to a concrete value.
    pub fn producer_seed_id(&self) -> Option<&SeedId> {
        optional_present_ref(&self.expectation.producer_seed_id)
    }

    /// Returns the expected artifact role, when the request constrains it.
    pub fn artifact_role(&self) -> Option<ArtifactRole> {
        self.expectation.artifact_role
    }

    /// Verifies returned evidence against this request.
    pub fn verify_evidence(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
        self.expectation.verify(evidence)
    }
}

fn optional_present_ref<T>(value: &OptionalEvidence<T>) -> Option<&T> {
    match value {
        OptionalEvidence::Present(value) => Some(value),
        OptionalEvidence::Any | OptionalEvidence::Absent => None,
    }
}

impl ArtifactEvidenceExpectation {
    fn verify(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
        if self.artifact_id != evidence.artifact_id {
            return mismatch(&self.artifact_id, "artifact_id");
        }
        if let Some(expected) = &self.digest {
            if expected != &evidence.digest {
                return mismatch(&self.artifact_id, "content_digest");
            }
        }
        if let Some(expected) = self.byte_len {
            if expected != evidence.byte_len {
                return mismatch(&self.artifact_id, "byte_len");
            }
        }
        if let Some(expected) = &self.media_type {
            if expected != &evidence.media_type {
                return mismatch(&self.artifact_id, "media_type");
            }
        }
        if !self.schema_id.matches(&evidence.schema_id) {
            return mismatch(&self.artifact_id, "schema_id");
        }
        if !self.semantic_type_id.matches(&evidence.semantic_type_id) {
            return mismatch(&self.artifact_id, "semantic_type_id");
        }
        if !self.producer_node_id.matches(&evidence.producer_node_id) {
            return mismatch(&self.artifact_id, "producer_node_id");
        }
        if !self.producer_seed_id.matches(&evidence.producer_seed_id) {
            return mismatch(&self.artifact_id, "producer_seed_id");
        }
        if let Some(expected) = self.artifact_role {
            if expected != evidence.artifact_role {
                return mismatch(&self.artifact_id, "artifact_role");
            }
        }
        Ok(())
    }
}

/// Verified artifact bytes and evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifactBytes {
    bytes: Vec<u8>,
    evidence: ArtifactEvidenceRef,
}

impl VerifiedArtifactBytes {
    /// Verifies bytes and evidence against the request before constructing a response.
    pub fn new(
        bytes: Vec<u8>,
        evidence: ArtifactEvidenceRef,
        request: &ArtifactReadRequest,
    ) -> Result<Self> {
        verify_bytes_match_evidence(&bytes, &evidence)?;
        request.verify_evidence(&evidence)?;
        Ok(Self { bytes, evidence })
    }

    /// Returns the verified artifact bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the response and returns verified artifact bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the verified artifact evidence.
    pub fn evidence(&self) -> &ArtifactEvidenceRef {
        &self.evidence
    }

    /// Decodes the verified bytes as JSON.
    pub fn decode_json<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.bytes).map_err(|_error| ArtifactReadError::Decode {
            artifact_id: Box::new(self.evidence.artifact_id.clone()),
            format: ArtifactReadDecodeFormat::Json,
        })
    }
}

/// Closed redaction-safe reasons for artifact-read requests that cannot produce bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactReadInvalidRequest {
    /// A skipped cell was requested as artifact bytes.
    SkippedCell,
}

/// Closed redaction-safe decoder formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactReadDecodeFormat {
    /// JSON decoding failed.
    Json,
}

/// Closed redaction-safe backend failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactReadBackendError {
    /// The backend failed without exposing endpoint, path, or secret details.
    Failed,
}

/// Redaction-safe artifact-read error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactReadError {
    /// Artifact bytes or evidence were not found.
    #[error("artifact {artifact_id} not found")]
    NotFound {
        /// Missing artifact id.
        artifact_id: Box<ArtifactId>,
    },
    /// Artifact bytes or evidence violated a request expectation.
    #[error("artifact {artifact_id} evidence mismatch for {field}")]
    EvidenceMismatch {
        /// Artifact id whose evidence failed verification.
        artifact_id: Box<ArtifactId>,
        /// Evidence field that failed verification.
        field: &'static str,
    },
    /// The request cannot be satisfied by artifact bytes.
    #[error("skipped cells do not have artifact bytes")]
    InvalidRequest {
        /// Closed redaction-safe reason.
        reason: ArtifactReadInvalidRequest,
    },
    /// Verified bytes could not be decoded.
    #[error("artifact {artifact_id} JSON decode failed")]
    Decode {
        /// Artifact id being decoded.
        artifact_id: Box<ArtifactId>,
        /// Closed redaction-safe decoder format.
        format: ArtifactReadDecodeFormat,
    },
    /// Backend failed without exposing endpoint, path, or secret details.
    #[error("typed artifact backend failed")]
    Backend {
        /// Closed redaction-safe backend reason.
        reason: ArtifactReadBackendError,
    },
}

impl ArtifactReadError {
    /// Builds a redacted backend failure, discarding source details.
    pub fn redacted_backend_failure(_source: impl fmt::Display) -> Self {
        Self::Backend {
            reason: ArtifactReadBackendError::Failed,
        }
    }
}

fn verify_bytes_match_evidence(bytes: &[u8], evidence: &ArtifactEvidenceRef) -> Result<()> {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    if digest != evidence.digest {
        return mismatch(&evidence.artifact_id, "content_digest");
    }
    let artifact_id = ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest());
    if artifact_id != evidence.artifact_id {
        return mismatch(&evidence.artifact_id, "artifact_id");
    }
    if evidence.byte_len != bytes.len() as u64 {
        return mismatch(&evidence.artifact_id, "byte_len");
    }
    Ok(())
}

fn optional_exact<T>(value: Option<T>) -> OptionalEvidence<T> {
    match value {
        Some(value) => OptionalEvidence::Present(value),
        None => OptionalEvidence::Absent,
    }
}

fn mismatch<T>(artifact_id: &ArtifactId, field: &'static str) -> Result<T> {
    Err(ArtifactReadError::EvidenceMismatch {
        artifact_id: Box::new(artifact_id.clone()),
        field,
    })
}
