#![warn(missing_docs)]
//! Certified typed execution spec contracts for MFM.
//!
//! This crate owns the persisted v1 typed execution spec shape and canonical spec-hash boundary
//! used by typed certification, runtime, replay, and storage.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{
    ContentDigest, ContextDescriptorId, ContextRef, ContextResourceKind, ContextStage,
    DigestAlgorithm, FieldPath as CheckedFieldPath, IdentityError,
    ResourceNamespace as CheckedResourceNamespace, SchemaId, SemanticTypeId, SpecHash,
    StableAuthorKey as CheckedStableAuthorKey, VisibleAscii256 as CheckedVisibleAscii256,
};

/// Result type for typed execution spec helpers.
pub type Result<T> = std::result::Result<T, SpecError>;

/// Error returned by typed execution spec helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// A stable string field failed validation.
    #[error("invalid {field} string {value:?}")]
    InvalidString {
        /// Field label.
        field: &'static str,
        /// Invalid value.
        value: String,
    },
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// JSON serialization failed before canonicalization.
    #[error("spec JSON serialization error: {0}")]
    Serialize(String),
    /// Persisted JSON decoding failed.
    #[error("spec JSON decoding error: {0}")]
    Json(String),
    /// Certified side-effect submit/verify pair resolution failed.
    #[error("side-effect verify pair error: {message}")]
    SideEffectVerifyPair {
        /// Machine-readable failure class.
        kind: SideEffectVerifyPairErrorKind,
        /// Stable diagnostic.
        message: String,
    },
    /// Canonical JSON construction failed.
    #[error("spec canonicalization error: {0}")]
    Canonical(String),
    /// Envelope hash did not match the canonical spec bytes.
    #[error("certified spec hash mismatch: expected {expected}, recomputed {actual}")]
    HashMismatch {
        /// Hash carried by the envelope.
        expected: Box<SpecHash>,
        /// Hash recomputed from the spec.
        actual: Box<SpecHash>,
    },
}

/// Machine-readable side-effect submit/verify pair resolution failure class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffectVerifyPairErrorKind {
    /// The requested node is not a side-effect verify framework node.
    NotVerifyNode,
    /// The verify node references a missing submit node.
    MissingSubmitNode,
    /// The verify node references a framework-owned node as its submit boundary.
    FrameworkSubmitNode,
    /// The verify node references a submit node without a side-effect contract.
    NonSideEffectSubmitNode,
    /// The verify node pair id is not derived from the submit node output cell and contract.
    PairIdMismatch,
}

impl From<IdentityError> for SpecError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_capabilities::CapabilityError> for SpecError {
    fn from(error: mfm_capabilities::CapabilityError) -> Self {
        Self::Identity(error.to_string())
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(&value).map_err(|error| SpecError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| SpecError::Canonical(error.to_string()))
}

fn content_digest(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json(value)?.content_digest())
}

/// Returns the schema id for canonical persisted v1 typed execution specs.
pub fn typed_execution_spec_schema_id() -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "fields": [
            "spec_version",
            "lowering_version",
            "config_refs",
            "contexts",
            "descriptor_identities",
            "nodes",
            "cells",
            "seeds",
            "edges",
            "outputs",
            "public_outputs",
            "saga_policy",
        ],
        "media_type": v1::MEDIA_TYPE,
        "name": "mfm.typed.execution_spec",
        "version": "1",
    }))?;
    Ok(SchemaId::new(
        "mfm.typed.execution_spec",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned schema id for public-output render receipts.
pub fn public_output_receipt_schema_id() -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "fields": [
            "public_schema_id",
            "output_spec_digest",
            "cells",
            "rendered_digest",
            "rendered_artifact_id",
            "renderer_descriptor_id",
        ],
        "name": "mfm.framework.public_output_receipt",
        "version": "1",
    }))?;
    Ok(SchemaId::new(
        "mfm.framework.public_output_receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned semantic type id for public-output render receipts.
pub fn public_output_receipt_semantic_type_id() -> Result<SemanticTypeId> {
    let digest = content_digest(serde_json::json!({
        "meaning": "framework public-output render receipt",
        "schema_id": public_output_receipt_schema_id()?.as_str(),
        "version": "1",
    }))?;
    Ok(SemanticTypeId::new(
        "mfm.framework.public_output",
        "receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned schema id for retention-manifest projection receipts.
pub fn retention_manifest_receipt_schema_id() -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "fields": [
            "manifest_seq",
            "manifest_digest",
            "previous_manifest_digest",
            "manifest_artifact_id",
            "pre_projection_stream_seq",
        ],
        "name": "mfm.framework.retention_manifest_receipt",
        "version": "1",
    }))?;
    Ok(SchemaId::new(
        "mfm.framework.retention_manifest_receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned semantic type id for retention-manifest projection receipts.
pub fn retention_manifest_receipt_semantic_type_id() -> Result<SemanticTypeId> {
    let digest = content_digest(serde_json::json!({
        "meaning": "framework retention manifest projection receipt",
        "schema_id": retention_manifest_receipt_schema_id()?.as_str(),
        "version": "1",
    }))?;
    Ok(SemanticTypeId::new(
        "mfm.framework.retention_manifest",
        "receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned schema id for complete-run receipts.
pub fn complete_run_receipt_schema_id() -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "fields": [
            "public_output_schema_id",
            "public_output_event_id",
            "completion_outcome",
        ],
        "name": "mfm.framework.complete_run_receipt",
        "version": "1",
    }))?;
    Ok(SchemaId::new(
        "mfm.framework.complete_run_receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned semantic type id for complete-run receipts.
pub fn complete_run_receipt_semantic_type_id() -> Result<SemanticTypeId> {
    let digest = content_digest(serde_json::json!({
        "meaning": "framework complete run receipt",
        "schema_id": complete_run_receipt_schema_id()?.as_str(),
        "version": "1",
    }))?;
    Ok(SemanticTypeId::new(
        "mfm.framework.complete_run",
        "receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned schema id for saga-terminal resolution receipts.
pub fn resolve_saga_terminal_receipt_schema_id() -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "fields": [
            "public_output_schema_id",
            "terminal_outcome",
            "pre_resolution_stream_seq",
        ],
        "name": "mfm.framework.resolve_saga_terminal_receipt",
        "version": "1",
    }))?;
    Ok(SchemaId::new(
        "mfm.framework.resolve_saga_terminal",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

/// Returns the framework-owned semantic type id for saga-terminal resolution receipts.
pub fn resolve_saga_terminal_receipt_semantic_type_id() -> Result<SemanticTypeId> {
    let digest = content_digest(serde_json::json!({
        "meaning": "framework saga-terminal resolution receipt",
        "schema_id": resolve_saga_terminal_receipt_schema_id()?.as_str(),
        "version": "1",
    }))?;
    Ok(SemanticTypeId::new(
        "mfm.framework.resolve_saga_terminal",
        "receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

fn spec_hash_from_canonical(canonical: &PlainCanonicalJsonBytes) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
}

macro_rules! checked_string_type {
    ($(#[$doc:meta])* $name:ident, $field:literal, $primitive:ty) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($primitive);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                <$primitive>::new(value).map(Self).map_err(|_| SpecError::InvalidString {
                    field: $field,
                    value: value.to_owned(),
                })
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Versioned v1 typed execution spec contracts.
pub mod v1;
