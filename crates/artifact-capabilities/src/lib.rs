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
    ) -> Self {
        Self::from_expectation(ArtifactEvidenceExpectation {
            artifact_id,
            digest: Some(digest),
            byte_len: None,
            media_type: None,
            schema_id: OptionalEvidence::Present(schema_id),
            semantic_type_id: OptionalEvidence::Present(semantic_type_id),
            producer_node_id: OptionalEvidence::Any,
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

    /// Verifies returned evidence against this request.
    pub fn verify_evidence(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
        self.expectation.verify(evidence)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactReadError {
    /// Artifact bytes or evidence were not found.
    NotFound {
        /// Missing artifact id.
        artifact_id: Box<ArtifactId>,
    },
    /// Artifact bytes or evidence violated a request expectation.
    EvidenceMismatch {
        /// Artifact id whose evidence failed verification.
        artifact_id: Box<ArtifactId>,
        /// Evidence field that failed verification.
        field: &'static str,
    },
    /// The request cannot be satisfied by artifact bytes.
    InvalidRequest {
        /// Closed redaction-safe reason.
        reason: ArtifactReadInvalidRequest,
    },
    /// Verified bytes could not be decoded.
    Decode {
        /// Artifact id being decoded.
        artifact_id: Box<ArtifactId>,
        /// Closed redaction-safe decoder format.
        format: ArtifactReadDecodeFormat,
    },
    /// Backend failed without exposing endpoint, path, or secret details.
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

impl fmt::Display for ArtifactReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { artifact_id } => write!(f, "artifact {artifact_id} not found"),
            Self::EvidenceMismatch { artifact_id, field } => {
                write!(f, "artifact {artifact_id} evidence mismatch for {field}")
            }
            Self::InvalidRequest { reason } => match reason {
                ArtifactReadInvalidRequest::SkippedCell => {
                    f.write_str("skipped cells do not have artifact bytes")
                }
            },
            Self::Decode {
                artifact_id,
                format,
            } => match format {
                ArtifactReadDecodeFormat::Json => {
                    write!(f, "artifact {artifact_id} JSON decode failed")
                }
            },
            Self::Backend { reason } => match reason {
                ArtifactReadBackendError::Failed => f.write_str("typed artifact backend failed"),
            },
        }
    }
}

impl std::error::Error for ArtifactReadError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
    }

    fn artifact_id(bytes: &[u8]) -> ArtifactId {
        let digest = digest(bytes);
        ArtifactId::from_digest(digest.algorithm(), *digest.digest())
    }

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

    fn seed_id(name: &str) -> SeedId {
        SeedId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(name.as_bytes()),
        )
    }

    fn evidence(bytes: &[u8]) -> ArtifactEvidenceRef {
        ArtifactEvidenceRef {
            artifact_id: artifact_id(bytes),
            digest: digest(bytes),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.schema")),
            semantic_type_id: Some(semantic_id("value")),
            producer_node_id: Some(node_id("producer")),
            producer_seed_id: None,
            artifact_role: ArtifactRole::StateOutput,
        }
    }

    fn request(evidence: &ArtifactEvidenceRef) -> ArtifactReadRequest {
        ArtifactReadRequest::from_replay_authorized_evidence(evidence.clone())
    }

    #[test]
    fn verifies_artifact_bytes() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = evidence(&bytes);

        let verified =
            VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), &request(&evidence))
                .expect("verified");

        assert_eq!(verified.bytes(), bytes.as_slice());
        assert_eq!(verified.evidence(), &evidence);
    }

    #[test]
    fn rejects_digest_mismatch() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let mut evidence = evidence(&bytes);
        evidence.digest = digest(br#"{"ok":false}"#);

        let err = VerifiedArtifactBytes::new(bytes, evidence.clone(), &request(&evidence))
            .expect_err("digest mismatch");

        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "content_digest",
                ..
            }
        ));
    }

    #[test]
    fn rejects_role_mismatch() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = evidence(&bytes);
        let mut expected = evidence.clone();
        expected.artifact_role = ArtifactRole::TypedConfig;
        let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

        let err = VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("role mismatch");

        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "artifact_role",
                ..
            }
        ));
    }

    #[test]
    fn materialized_seed_cell_request_checks_seed_role_schema_and_semantic_identity() {
        let bytes = br#"{"seed":true}"#.to_vec();
        let seed_id = seed_id("configured");
        let request = ArtifactReadRequest::from_materialized_seed_cell(
            artifact_id(&bytes),
            digest(&bytes),
            schema_id("mfm.test.seed_schema"),
            semantic_id("seed_value"),
            seed_id.clone(),
        );
        let evidence = ArtifactEvidenceRef {
            artifact_id: artifact_id(&bytes),
            digest: digest(&bytes),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.seed_schema")),
            semantic_type_id: Some(semantic_id("seed_value")),
            producer_node_id: None,
            producer_seed_id: Some(seed_id),
            artifact_role: ArtifactRole::SeedInput,
        };

        VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified seed cell");
    }

    #[test]
    fn materialized_produced_cell_request_checks_state_output_schema_and_semantic_identity() {
        let bytes = br#"{"produced":true}"#.to_vec();
        let request = ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id(&bytes),
            digest(&bytes),
            schema_id("mfm.test.output_schema"),
            semantic_id("output_value"),
        );
        let evidence = ArtifactEvidenceRef {
            artifact_id: artifact_id(&bytes),
            digest: digest(&bytes),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.output_schema")),
            semantic_type_id: Some(semantic_id("output_value")),
            producer_node_id: Some(node_id("producer")),
            producer_seed_id: None,
            artifact_role: ArtifactRole::StateOutput,
        };

        VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified produced cell");
    }

    #[test]
    fn side_effect_projection_request_accepts_unprojected_schema_and_semantic_identity() {
        let bytes = br#"{"prepared":true}"#.to_vec();
        let projection = store::SideEffectArtifactProjection {
            artifact_id: artifact_id(&bytes),
            content_digest: digest(&bytes),
            schema_id: None,
        };
        let request = ArtifactReadRequest::from_side_effect_projection(
            &projection,
            ArtifactRole::PreparedInvocation,
            node_id("producer"),
        );
        let evidence = ArtifactEvidenceRef {
            artifact_id: artifact_id(&bytes),
            digest: digest(&bytes),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.prepared_schema")),
            semantic_type_id: Some(semantic_id("prepared_value")),
            producer_node_id: Some(node_id("producer")),
            producer_seed_id: None,
            artifact_role: ArtifactRole::PreparedInvocation,
        };

        VerifiedArtifactBytes::new(bytes, evidence, &request)
            .expect("verified prepared side-effect artifact");
    }

    #[test]
    fn rejects_schema_and_semantic_mismatch() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = evidence(&bytes);
        let mut expected = evidence.clone();
        expected.schema_id = Some(schema_id("mfm.test.other_schema"));
        let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

        let err = VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), &request)
            .expect_err("schema mismatch");
        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "schema_id",
                ..
            }
        ));

        let mut expected = evidence.clone();
        expected.semantic_type_id = Some(semantic_id("other_value"));
        let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

        let err =
            VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("semantic mismatch");
        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "semantic_type_id",
                ..
            }
        ));
    }

    #[test]
    fn rejects_producer_mismatch() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = evidence(&bytes);
        let mut expected = evidence.clone();
        expected.producer_node_id = Some(node_id("other-producer"));
        let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

        let err =
            VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("producer mismatch");

        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "producer_node_id",
                ..
            }
        ));
    }

    #[test]
    fn rejects_byte_length_mismatch() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let mut evidence = evidence(&bytes);
        evidence.byte_len += 1;

        let err = VerifiedArtifactBytes::new(bytes, evidence.clone(), &request(&evidence))
            .expect_err("byte length mismatch");

        assert!(matches!(
            err,
            ArtifactReadError::EvidenceMismatch {
                field: "byte_len",
                ..
            }
        ));
    }

    #[test]
    fn errors_are_redaction_safe() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let evidence = evidence(&bytes);
        let err = ArtifactReadError::redacted_backend_failure("/tmp/secret/mfm-key.json");
        let rendered = err.to_string();

        assert!(matches!(
            err,
            ArtifactReadError::Backend {
                reason: ArtifactReadBackendError::Failed
            }
        ));
        assert!(!rendered.contains("/tmp/secret"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("private_key"));
        assert!(!rendered.contains(String::from_utf8_lossy(&bytes).as_ref()));
        assert!(!rendered.contains(evidence.digest.as_str()));
    }
}
