#![warn(missing_docs)]
//! Certified typed execution spec contracts for MFM.
//!
//! This crate owns the persisted v1 typed execution spec shape and canonical spec-hash boundary
//! used by typed certification, runtime, replay, and storage.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, IdentityError, SchemaId, SemanticTypeId, SpecHash};

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
    /// Obsolete pre-v1 sketch shape was detected.
    #[error("obsolete spec shape: {0}")]
    ObsoleteSketchShape(String),
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

fn checked_ascii_token(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    {
        return Err(SpecError::InvalidString {
            field,
            value: value.to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn checked_author_key(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.is_empty()
        || value.len() > 256
        || value.starts_with("mfm.")
        || value.starts_with("sys.")
        || value.starts_with('_')
        || !value.split('/').all(is_valid_author_key_segment)
    {
        return Err(SpecError::InvalidString {
            field,
            value: value.to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn is_valid_author_key_segment(segment: &str) -> bool {
    if segment.is_empty() || segment.len() > 64 {
        return false;
    }
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
        })
}

fn checked_field_path(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.split('.').all(is_valid_field_segment) {
        Ok(value.to_owned())
    } else {
        Err(SpecError::InvalidString {
            field,
            value: value.to_owned(),
        })
    }
}

fn is_valid_field_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/'))
}

fn checked_resource_namespace(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.len() <= 256
        && value.contains('.')
        && value.split('.').all(is_valid_resource_namespace_segment)
    {
        Ok(value.to_owned())
    } else {
        Err(SpecError::InvalidString {
            field,
            value: value.to_owned(),
        })
    }
}

fn is_valid_resource_namespace_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-'))
}

macro_rules! checked_string_type {
    ($(#[$doc:meta])* $name:ident, $field:literal, $checker:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                $checker($field, value).map(Self)
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                &self.0
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
pub mod v1 {
    use std::collections::BTreeMap;

    use super::{
        canonical_json, checked_ascii_token, checked_author_key, checked_field_path,
        checked_resource_namespace, content_digest, spec_hash_from_canonical, ContentDigest,
        DigestAlgorithm, PlainCanonicalJsonBytes, Result, SideEffectVerifyPairErrorKind, SpecError,
        SpecHash,
    };
    use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, CellId, DescriptorId, EffectKind, EffectVersion,
        LoweringVersion, NodeId, OperationInstanceId, OperationKind, OperationVersion, SchemaId,
        ScopeId, SeedId, SemanticTypeId, SideEffectPairId, SpecVersion, StateKind, StateVersion,
    };

    /// v1 spec-version string.
    pub const SPEC_VERSION: &str = "mfm.typed.execution_spec.v1";
    /// v1 typed execution spec media type.
    pub const MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";
    /// v1 lowering-version string.
    pub const LOWERING_VERSION: &str = "mfm.typed.lowering.v1";

    /// Returns the schema id for canonical persisted v1 typed execution specs.
    pub fn typed_execution_spec_schema_id() -> Result<SchemaId> {
        super::typed_execution_spec_schema_id()
    }

    /// Returns the v1 framework-owned schema id for public-output render receipts.
    pub fn public_output_receipt_schema_id() -> Result<SchemaId> {
        super::public_output_receipt_schema_id()
    }

    /// Returns the v1 framework-owned semantic type id for public-output render receipts.
    pub fn public_output_receipt_semantic_type_id() -> Result<SemanticTypeId> {
        super::public_output_receipt_semantic_type_id()
    }

    /// Returns the v1 framework-owned schema id for retention-manifest projection receipts.
    pub fn retention_manifest_receipt_schema_id() -> Result<SchemaId> {
        super::retention_manifest_receipt_schema_id()
    }

    /// Returns the v1 framework-owned semantic type id for retention-manifest projection receipts.
    pub fn retention_manifest_receipt_semantic_type_id() -> Result<SemanticTypeId> {
        super::retention_manifest_receipt_semantic_type_id()
    }

    /// Returns the v1 framework-owned schema id for complete-run receipts.
    pub fn complete_run_receipt_schema_id() -> Result<SchemaId> {
        super::complete_run_receipt_schema_id()
    }

    /// Returns the v1 framework-owned semantic type id for complete-run receipts.
    pub fn complete_run_receipt_semantic_type_id() -> Result<SemanticTypeId> {
        super::complete_run_receipt_semantic_type_id()
    }

    /// Returns the v1 framework-owned schema id for saga-terminal resolution receipts.
    pub fn resolve_saga_terminal_receipt_schema_id() -> Result<SchemaId> {
        super::resolve_saga_terminal_receipt_schema_id()
    }

    /// Returns the v1 framework-owned semantic type id for saga-terminal resolution receipts.
    pub fn resolve_saga_terminal_receipt_semantic_type_id() -> Result<SemanticTypeId> {
        super::resolve_saga_terminal_receipt_semantic_type_id()
    }

    /// Returns canonical JSON bytes for a framework-owned config artifact.
    pub fn framework_config_canonical_json(
        kind: &str,
        node_id: &NodeId,
    ) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(serde_json::json!({
            "framework": kind,
            "node_id": node_id.as_str(),
        }))
    }

    /// Returns the deterministic framework-owned config reference for a framework node.
    pub fn framework_config_ref(kind: &str, node_id: &NodeId) -> Result<ConfigRef> {
        let bytes = framework_config_canonical_json(kind, node_id)?;
        let digest = bytes.content_digest();
        let schema_digest = content_digest(serde_json::json!({ "framework": kind }))?;
        Ok(ConfigRef {
            schema_id: SchemaId::new(
                "mfm.framework.config",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                *schema_digest.digest(),
            )?,
            artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
            digest,
            byte_len: bytes.as_bytes().len() as u64,
            media_type: MediaType::new("application/json")?,
        })
    }

    /// Derives the certified side-effect pair id for one submit node and contract.
    pub fn side_effect_pair_id(
        submit_node_id: &NodeId,
        submit_output_cell_id: &CellId,
        contract: &SideEffectContractSpec,
    ) -> Result<SideEffectPairId> {
        Ok(SideEffectPairId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *content_digest(serde_json::json!({
                "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
                "lowering_version": LOWERING_VERSION,
                "side_effect_contract": contract.json(),
                "submit_node_id": submit_node_id.as_str(),
                "submit_output_cell_id": submit_output_cell_id.as_str(),
            }))?
            .digest(),
        ))
    }

    /// Derives the framework verify node id for a certified side-effect pair.
    pub fn side_effect_verify_node_id(
        submit_node_id: &NodeId,
        pair_id: &SideEffectPairId,
    ) -> Result<NodeId> {
        Ok(NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *content_digest(serde_json::json!({
                "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
                "framework": "side_effect_verify",
                "lowering_version": LOWERING_VERSION,
                "pair_id": pair_id.as_str(),
                "submit_node_id": submit_node_id.as_str(),
            }))?
            .digest(),
        ))
    }

    /// Derives the framework verify stable key for a side-effect submit key.
    pub fn side_effect_verify_stable_key(submit_key: &StableAuthorKey) -> Result<StableAuthorKey> {
        StableAuthorKey::new(format!(
            "framework/side-effect-verify/{}",
            submit_key.as_str()
        ))
    }

    /// Returns the deterministic unit input binding for a lifecycle framework node.
    pub fn framework_lifecycle_unit_input_binding(kind: &str) -> Result<InputBindingSpec> {
        let root = InputBindingNodeSpec::Unit;
        Ok(InputBindingSpec {
            input_schema_id: framework_lifecycle_unit_input_schema_id(kind)?,
            input_descriptor_id: framework_lifecycle_input_descriptor_id(
                kind,
                serde_json::json!({ "input": "unit" }),
            )?,
            digest: framework_lifecycle_input_digest(&root)?,
            root,
        })
    }

    /// Returns the deterministic receipt-cell input binding for a lifecycle framework node.
    pub fn framework_lifecycle_receipt_input_binding(
        kind: &str,
        field_path: &str,
        cell: &CellSpec,
    ) -> Result<InputBindingSpec> {
        framework_lifecycle_cell_input_binding(
            kind,
            field_path,
            cell,
            RequiredTerminal::ProducedOnly,
            "receipt_cell",
        )
    }

    /// Returns the deterministic maybe-skipped cell input binding for a lifecycle framework node.
    pub fn framework_lifecycle_maybe_skipped_cell_input_binding(
        kind: &str,
        field_path: &str,
        cell: &CellSpec,
    ) -> Result<InputBindingSpec> {
        framework_lifecycle_cell_input_binding(
            kind,
            field_path,
            cell,
            RequiredTerminal::MaybeSkipped,
            "maybe_skipped_cell",
        )
    }

    fn framework_lifecycle_cell_input_binding(
        kind: &str,
        field_path: &str,
        cell: &CellSpec,
        required_terminal: RequiredTerminal,
        input_kind: &str,
    ) -> Result<InputBindingSpec> {
        let field_path = PublicFieldPath::new(field_path)?;
        let root = InputBindingNodeSpec::Cell(Box::new(InputBindingCellSpec {
            field_path: field_path.clone(),
            cell_id: cell.cell_id.clone(),
            semantic_type_id: cell.semantic_type_id.clone(),
            schema_id: cell.schema_id.clone(),
            required_terminal,
            value_lineage: cell.value_lineage.clone(),
        }));
        Ok(InputBindingSpec {
            input_schema_id: cell.schema_id.clone(),
            input_descriptor_id: framework_lifecycle_input_descriptor_id(
                kind,
                serde_json::json!({
                    "cell_id": cell.cell_id.as_str(),
                    "field_path": field_path.as_str(),
                    "input": input_kind,
                    "schema_id": cell.schema_id.as_str(),
                    "semantic_type_id": cell.semantic_type_id.as_str(),
                }),
            )?,
            digest: framework_lifecycle_input_digest(&root)?,
            root,
        })
    }

    fn framework_lifecycle_input_digest(root: &InputBindingNodeSpec) -> Result<ContentDigest> {
        content_digest(framework_lifecycle_input_digest_json(root))
    }

    fn framework_lifecycle_input_digest_json(root: &InputBindingNodeSpec) -> serde_json::Value {
        match root {
            InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
            InputBindingNodeSpec::Cell(cell) => serde_json::json!({
                "cell_id": cell.cell_id.as_str(),
                "field_path": cell.field_path.as_str(),
                "kind": "cell",
                "required_terminal": cell.required_terminal.as_str(),
                "schema_id": cell.schema_id.as_str(),
                "semantic_type_id": cell.semantic_type_id.as_str(),
                "value_lineage": cell.value_lineage.lineage_digest.as_str(),
            }),
            InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
                "elements": elements
                    .iter()
                    .map(framework_lifecycle_input_digest_json)
                    .collect::<Vec<_>>(),
                "kind": "tuple",
            }),
            InputBindingNodeSpec::Struct(fields) => serde_json::json!({
                "fields": fields
                    .iter()
                    .map(|field| serde_json::json!({
                        "field_path": field.field_path.as_str(),
                        "node": framework_lifecycle_input_digest_json(&field.node),
                    }))
                    .collect::<Vec<_>>(),
                "kind": "struct",
            }),
            InputBindingNodeSpec::Vec {
                elements,
                ordering,
                domain_keys,
            } => serde_json::json!({
                "domain_keys": domain_keys
                    .iter()
                    .map(|key| serde_json::json!({
                        "content_digest": key.content_digest.as_str(),
                        "schema_id": key.schema_id.as_str(),
                    }))
                    .collect::<Vec<_>>(),
                "elements": elements
                    .iter()
                    .map(framework_lifecycle_input_digest_json)
                    .collect::<Vec<_>>(),
                "kind": "vec",
                "ordering": ordering.as_str(),
            }),
            InputBindingNodeSpec::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            } => serde_json::json!({
                "domain_keys": domain_keys
                    .iter()
                    .map(|key| serde_json::json!({
                        "content_digest": key.content_digest.as_str(),
                        "schema_id": key.schema_id.as_str(),
                    }))
                    .collect::<Vec<_>>(),
                "elements": elements
                    .iter()
                    .map(framework_lifecycle_input_digest_json)
                    .collect::<Vec<_>>(),
                "kind": "non_empty_vec",
                "ordering": ordering.as_str(),
            }),
        }
    }

    fn framework_lifecycle_unit_input_schema_id(kind: &str) -> Result<SchemaId> {
        let digest = content_digest(serde_json::json!({
            "framework": kind,
            "input": "unit",
        }))?;
        Ok(SchemaId::new(
            "mfm.framework.lifecycle_unit_input",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        )?)
    }

    fn framework_lifecycle_input_descriptor_id(
        kind: &str,
        input: serde_json::Value,
    ) -> Result<DescriptorId> {
        Ok(DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *content_digest(serde_json::json!({
                "framework": kind,
                "input": input,
            }))?
            .digest(),
        ))
    }

    checked_string_type!(
        /// Checked media type string.
        MediaType,
        "media type",
        checked_ascii_token
    );
    checked_string_type!(
        /// Stable author key persisted in specs.
        StableAuthorKey,
        "stable author key",
        checked_author_key
    );
    checked_string_type!(
        /// Public output field path persisted in specs.
        PublicFieldPath,
        "public field path",
        checked_field_path
    );
    checked_string_type!(
        /// Stable renderer kind string.
        RendererKind,
        "renderer kind",
        checked_author_key
    );
    checked_string_type!(
        /// Stable renderer version string.
        RendererVersion,
        "renderer version",
        checked_ascii_token
    );
    checked_string_type!(
        /// Canonicalizer identity string used by a renderer.
        CanonicalizerIdentity,
        "canonicalizer identity",
        checked_ascii_token
    );
    checked_string_type!(
        /// Capability-style resource namespace for cross-run resource claims.
        ResourceNamespace,
        "resource namespace",
        checked_resource_namespace
    );
    checked_string_type!(
        /// Manual authorization verifier identity certified for manual saga resolution.
        ManualAuthorizationVerifierId,
        "manual authorization verifier id",
        checked_ascii_token
    );
    checked_string_type!(
        /// Operator authority snapshot identity certified for manual saga resolution.
        OperatorAuthorityId,
        "operator authority id",
        checked_ascii_token
    );
    checked_string_type!(
        /// Stable operator identity inside a certified operator authority snapshot.
        OperatorId,
        "operator id",
        checked_ascii_token
    );
    checked_string_type!(
        /// Public operator identity that may authorize a manual saga resolution.
        OperatorPublicIdentity,
        "operator public identity",
        checked_ascii_token
    );
    checked_string_type!(
        /// Manual-resolution signing scheme identifier.
        ManualSigningSchemeSpec,
        "manual signing scheme",
        checked_ascii_token
    );

    /// Parsed or constructed typed spec data that has not been certified.
    ///
    /// This wrapper is a hostile persistence/interop boundary. It proves only that the contained
    /// data decoded into typed Rust values and can be re-hashed; it is not runtime authority.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UntrustedTypedSpec {
        spec: TypedExecutionSpec,
    }

    impl UntrustedTypedSpec {
        /// Wraps raw typed spec data without minting certification authority.
        pub fn from_raw_spec(spec: TypedExecutionSpec) -> Self {
            Self { spec }
        }

        /// Decodes untrusted persisted v1 typed execution spec JSON.
        pub fn from_json_str(input: &str) -> Result<Self> {
            Ok(Self::from_raw_spec(TypedExecutionSpec::from_json_str(
                input,
            )?))
        }

        /// Decodes untrusted persisted v1 typed execution spec UTF-8 JSON bytes.
        pub fn from_json_slice(input: &[u8]) -> Result<Self> {
            Ok(Self::from_raw_spec(TypedExecutionSpec::from_json_slice(
                input,
            )?))
        }

        /// Returns the hostile typed spec data.
        pub fn spec(&self) -> &TypedExecutionSpec {
            &self.spec
        }

        /// Consumes the wrapper and returns the hostile typed spec data.
        pub fn into_raw_spec(self) -> TypedExecutionSpec {
            self.spec
        }
    }

    /// Hash-only envelope carrying a spec hash and non-semantic audit metadata.
    ///
    /// This type is not certification authority. It only proves that `spec_hash` matches
    /// the canonical bytes of `spec`; runtime authority must come from `mfm-certify`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct HashedSpecEnvelope {
        /// Canonical hash of `spec` only.
        pub spec_hash: SpecHash,
        /// Hash-defining typed execution spec.
        pub spec: TypedExecutionSpec,
        /// Non-semantic audit metadata.
        pub audit: TypedExecutionSpecAudit,
    }

    impl HashedSpecEnvelope {
        /// Builds an envelope by hashing the supplied spec.
        pub fn new(spec: TypedExecutionSpec, audit: TypedExecutionSpecAudit) -> Result<Self> {
            let spec_hash = spec.spec_hash()?;
            Ok(Self {
                spec_hash,
                spec,
                audit,
            })
        }

        /// Verifies that the stored hash still matches the canonical spec bytes.
        pub fn verify_hash(&self) -> Result<()> {
            let actual = self.spec.spec_hash()?;
            if self.spec_hash != actual {
                return Err(SpecError::HashMismatch {
                    expected: Box::new(self.spec_hash.clone()),
                    actual: Box::new(actual),
                });
            }
            Ok(())
        }
    }

    /// Hash-defining v1 typed execution spec data.
    ///
    /// Parsed or directly constructed values of this type are not certification or runtime
    /// authority. Pass them through [`UntrustedTypedSpec`] and `mfm-certify` to obtain validated
    /// and certified authority.
    ///
    /// Saga policy is certified spec data because external side-effect remediation is a
    /// saga-only claim derived from certified policy plus recorded stream facts. Directive
    /// selection, obligation state, manual blocking, and run mode are not spec-authored control
    /// events; they are rebuildable projections.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TypedExecutionSpec {
        /// Spec contract version.
        pub spec_version: SpecVersion,
        /// Persisted media type.
        pub media_type: MediaType,
        /// Canonicalization and hash algorithm.
        pub canonicalization: DigestAlgorithm,
        /// Lowering algorithm version.
        pub lowering_version: LoweringVersion,
        /// Authoring provenance.
        pub authoring: AuthoringProvenance,
        /// Certified run-level saga policy.
        pub saga: SagaPolicySpec,
        /// Persisted scopes.
        pub scopes: Vec<ScopeSpec>,
        /// Declared seed cells.
        pub seeds: Vec<SeedSpec>,
        /// Descriptor identities required by certification.
        pub descriptor_identities: Vec<DescriptorIdentity>,
        /// Config artifacts referenced by nodes and lineage.
        pub config_refs: Vec<ConfigRef>,
        /// State and framework nodes.
        pub nodes: Vec<NodeSpec>,
        /// Remediation nodes keyed by the forward side-effect node they compensate.
        pub remediations: BTreeMap<NodeId, NodeSpec>,
        /// Planned cells.
        pub cells: Vec<CellSpec>,
        /// Hash-defining value lineage records referenced by cells and inputs.
        pub value_lineages: Vec<ValueLineage>,
        /// Registry-mediated planning lineage frames.
        pub planning_lineage: Vec<OperationLineageFrameSpec>,
        /// Public output contract.
        pub public_outputs: PublicOutputSpec,
    }

    impl TypedExecutionSpec {
        /// Creates a v1 spec with canonical version, media type, canonicalization, and lowering ids.
        pub fn new(parts: TypedExecutionSpecParts) -> Result<Self> {
            Ok(Self {
                spec_version: SpecVersion::new(SPEC_VERSION)?,
                media_type: MediaType::new(MEDIA_TYPE)?,
                canonicalization: DigestAlgorithm::Sha256JcsV1,
                lowering_version: LoweringVersion::new(LOWERING_VERSION)?,
                authoring: parts.authoring,
                saga: parts.saga,
                scopes: parts.scopes,
                seeds: parts.seeds,
                descriptor_identities: parts.descriptor_identities,
                config_refs: parts.config_refs,
                nodes: parts.nodes,
                remediations: parts.remediations,
                cells: parts.cells,
                value_lineages: parts.value_lineages,
                planning_lineage: parts.planning_lineage,
                public_outputs: parts.public_outputs,
            })
        }

        /// Returns canonical JSON bytes for the hash-defining spec.
        pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
            canonical_json(self.json()?)
        }

        /// Returns the spec hash over canonical `TypedExecutionSpec` bytes.
        pub fn spec_hash(&self) -> Result<SpecHash> {
            Ok(spec_hash_from_canonical(&self.canonical_json()?))
        }

        /// Rejects obsolete pre-v1 sketch objects that used `state_program` or `outputs`.
        pub fn validate_persisted_json_shape(input: &str) -> Result<()> {
            let value: serde_json::Value = serde_json::from_str(input)
                .map_err(|error| SpecError::Serialize(error.to_string()))?;
            let Some(object) = value.as_object() else {
                return Err(SpecError::ObsoleteSketchShape(
                    "typed execution spec must be a JSON object".to_owned(),
                ));
            };
            if object.contains_key("state_program") {
                return Err(SpecError::ObsoleteSketchShape(
                    "state_program is not part of the v1 contract".to_owned(),
                ));
            }
            if object.contains_key("outputs") {
                return Err(SpecError::ObsoleteSketchShape(
                    "outputs is obsolete; use public_outputs".to_owned(),
                ));
            }
            Ok(())
        }

        /// Decodes a persisted v1 typed execution spec from canonical or non-canonical JSON.
        ///
        /// The returned value is reconstructed through checked typed constructors and can be
        /// re-hashed through [`Self::spec_hash`]. Callers that load a run-start artifact must
        /// compare the recomputed hash with the `RunAdmitted.spec_hash` stored in the typed stream.
        pub fn from_json_str(input: &str) -> Result<Self> {
            Self::validate_persisted_json_shape(input)?;
            let input_canonical = PlainCanonicalJsonBytes::from_json_str(input)
                .map_err(|error| SpecError::Canonical(error.to_string()))?;
            let value: serde_json::Value =
                serde_json::from_str(input).map_err(|error| SpecError::Json(error.to_string()))?;
            let spec = parse_typed_execution_spec(&value)?;
            let parsed_canonical = spec.canonical_json()?;
            if parsed_canonical != input_canonical {
                return Err(SpecError::Json(
                    "persisted typed execution spec contains unknown or non-normalized fields"
                        .to_owned(),
                ));
            }
            Ok(spec)
        }

        /// Decodes a persisted v1 typed execution spec from UTF-8 JSON bytes.
        pub fn from_json_slice(input: &[u8]) -> Result<Self> {
            let input = std::str::from_utf8(input)
                .map_err(|error| SpecError::Json(format!("spec JSON is not UTF-8: {error}")))?;
            Self::from_json_str(input)
        }

        /// Resolves a certified side-effect verify pair by verify node id.
        pub fn side_effect_verify_pair_for_verify_node(
            &self,
            verify_node_id: &NodeId,
        ) -> Result<SideEffectVerifyPairRef<'_>> {
            let verify_node = self
                .nodes
                .iter()
                .find(|node| &node.node_id == verify_node_id)
                .ok_or_else(|| {
                    json_error(format!(
                        "side-effect verify node {verify_node_id} is not present in spec"
                    ))
                })?;
            self.resolve_side_effect_verify_pair(verify_node)
        }

        /// Resolves a certified side-effect verify pair by pair id.
        pub fn side_effect_verify_pair_for_pair_id(
            &self,
            pair_id: &SideEffectPairId,
        ) -> Result<SideEffectVerifyPairRef<'_>> {
            let mut matches = self.nodes.iter().filter(|node| {
                matches!(
                    &node.framework,
                    Some(FrameworkNodeSpec::SideEffectVerify(verify))
                        if verify.pair_id == *pair_id
                )
            });
            let verify_node = matches.next().ok_or_else(|| {
                json_error(format!(
                    "side-effect pair {pair_id} has no certified verify node"
                ))
            })?;
            if matches.next().is_some() {
                return Err(json_error(format!(
                    "side-effect pair {pair_id} has multiple certified verify nodes"
                )));
            }
            self.resolve_side_effect_verify_pair(verify_node)
        }

        /// Resolves a certified side-effect verify pair by submit node id.
        pub fn side_effect_verify_pair_for_submit_node(
            &self,
            submit_node_id: &NodeId,
        ) -> Result<SideEffectVerifyPairRef<'_>> {
            let mut matches = self.nodes.iter().filter(|node| {
                matches!(
                    &node.framework,
                    Some(FrameworkNodeSpec::SideEffectVerify(verify))
                        if verify.submit_node_id == *submit_node_id
                )
            });
            let verify_node = matches.next().ok_or_else(|| {
                json_error(format!(
                    "submit node {submit_node_id} is missing certified side-effect verify node"
                ))
            })?;
            if matches.next().is_some() {
                return Err(json_error(format!(
                    "submit node {submit_node_id} has multiple certified side-effect verify nodes"
                )));
            }
            self.resolve_side_effect_verify_pair(verify_node)
        }

        fn resolve_side_effect_verify_pair<'a>(
            &'a self,
            verify_node: &'a NodeSpec,
        ) -> Result<SideEffectVerifyPairRef<'a>> {
            resolve_side_effect_verify_pair(&self.nodes, &self.remediations, verify_node)
        }

        fn json(&self) -> Result<serde_json::Value> {
            let descriptor_index = DescriptorJsonIndex::new(&self.descriptor_identities)?;
            Ok(serde_json::json!({
                "authoring": self.authoring.json(),
                "canonicalization": self.canonicalization.as_str(),
                "cells": self.cells.iter().map(CellSpec::json).collect::<Vec<_>>(),
                "config_refs": self.config_refs.iter().map(ConfigRef::json).collect::<Vec<_>>(),
                "descriptor_identities": self.descriptor_identities
                    .iter()
                    .map(DescriptorIdentity::json)
                    .collect::<Vec<_>>(),
                "lowering_version": self.lowering_version.as_str(),
                "media_type": self.media_type.as_str(),
                "nodes": self.nodes
                    .iter()
                    .map(|node| node.json(&descriptor_index))
                    .collect::<Result<Vec<_>>>()?,
                "planning_lineage": self.planning_lineage
                    .iter()
                    .map(|frame| frame.json(&descriptor_index))
                    .collect::<Result<Vec<_>>>()?,
                "public_outputs": self.public_outputs.json()?,
                "remediations": remediations_json(&self.remediations, &descriptor_index)?,
                "saga": self.saga.json(),
                "scopes": self.scopes.iter().map(ScopeSpec::json).collect::<Vec<_>>(),
                "seeds": self.seeds.iter().map(SeedSpec::json).collect::<Vec<_>>(),
                "spec_version": self.spec_version.as_str(),
                "value_lineages": self.value_lineages.iter().map(ValueLineage::json).collect::<Vec<_>>(),
            }))
        }
    }

    /// Certified run-level saga policy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SagaPolicySpec {
        /// Lowering uses this when the forward graph has no side-effect nodes.
        NoSideEffects,
        /// Failure after mutation carries no compensation or AC/DC-equivalence claim.
        FailWithoutAcdcClaim,
        /// Failure after mutation blocks for typed operator evidence.
        ManualResolution {
            /// Required typed manual evidence.
            manual: ManualResolutionEvidenceSpec,
        },
        /// Failure after confirmed forward side effects compensates linked remediations.
        CompensateCompleted {
            /// Directive used when a forward or remediation ledger remains unresolved.
            on_remediation_unresolved: RemediationUnresolvedSpec,
        },
    }

    impl SagaPolicySpec {
        /// Returns the canonical digest of this saga policy.
        pub fn saga_policy_digest(&self) -> Result<ContentDigest> {
            content_digest(self.json())
        }

        fn json(&self) -> serde_json::Value {
            match self {
                Self::NoSideEffects => serde_json::json!({
                    "kind": "no_side_effects",
                }),
                Self::FailWithoutAcdcClaim => serde_json::json!({
                    "kind": "fail_without_acdc_claim",
                }),
                Self::ManualResolution { manual } => serde_json::json!({
                    "kind": "manual_resolution",
                    "manual": manual.json(),
                }),
                Self::CompensateCompleted {
                    on_remediation_unresolved,
                } => serde_json::json!({
                    "kind": "compensate_completed",
                    "on_remediation_unresolved": on_remediation_unresolved.json(),
                }),
            }
        }
    }

    /// Certified directive for unresolved remediation under compensating policy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum RemediationUnresolvedSpec {
        /// Block for typed operator evidence.
        ManualResolution {
            /// Required typed manual evidence.
            manual: Box<ManualResolutionEvidenceSpec>,
        },
        /// Terminally fail without a compensation or AC/DC-equivalence claim.
        FailWithoutAcdcClaim,
    }

    impl RemediationUnresolvedSpec {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::ManualResolution { manual } => serde_json::json!({
                    "kind": "manual_resolution",
                    "manual": manual.json(),
                }),
                Self::FailWithoutAcdcClaim => serde_json::json!({
                    "kind": "fail_without_acdc_claim",
                }),
            }
        }
    }

    /// Typed schema requirements for run-scoped manual saga resolution evidence.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManualResolutionEvidenceSpec {
        /// Schema id for the operator evidence artifact.
        pub evidence_schema: SchemaId,
        /// Certified authorization policy required for the manual decision.
        pub authorization: ManualResolutionAuthorizationSpec,
    }

    impl ManualResolutionEvidenceSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "authorization": self.authorization.json(),
                "evidence_schema": self.evidence_schema.as_str(),
            })
        }
    }

    /// Certified authorization policy for run-scoped manual saga resolution.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManualResolutionAuthorizationSpec {
        /// Verifier that must validate the authorization proof.
        pub verifier_id: ManualAuthorizationVerifierId,
        /// Digest-only signing scheme used for manual-resolution claims.
        pub signing_scheme: ManualSigningSchemeSpec,
        /// Certified operator authority snapshot.
        pub authority: OperatorAuthoritySnapshotSpec,
        /// Required authorization quorum.
        pub quorum: ManualAuthorizationQuorumSpec,
    }

    impl ManualResolutionAuthorizationSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "authority": self.authority.json(),
                "quorum": self.quorum.json(),
                "signing_scheme": self.signing_scheme.as_str(),
                "verifier_id": self.verifier_id.as_str(),
            })
        }
    }

    /// Certified operator authority snapshot for manual saga resolution.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OperatorAuthoritySnapshotSpec {
        /// Authority snapshot identity.
        pub authority_id: OperatorAuthorityId,
        /// Operators allowed by this snapshot.
        pub operators: Vec<OperatorAuthorityMemberSpec>,
    }

    impl OperatorAuthoritySnapshotSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "authority_id": self.authority_id.as_str(),
                "operators": self
                    .operators
                    .iter()
                    .map(OperatorAuthorityMemberSpec::json)
                    .collect::<Vec<_>>(),
            })
        }
    }

    /// Operator authorized by a certified manual authority snapshot.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OperatorAuthorityMemberSpec {
        /// Stable operator id inside the authority snapshot.
        pub operator_id: OperatorId,
        /// Public signing identity for this operator.
        pub public_identity: OperatorPublicIdentity,
    }

    impl OperatorAuthorityMemberSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "operator_id": self.operator_id.as_str(),
                "public_identity": self.public_identity.as_str(),
            })
        }
    }

    /// Manual authorization quorum for a signed manual decision.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManualAuthorizationQuorumSpec {
        required_signatures: u32,
    }

    impl ManualAuthorizationQuorumSpec {
        /// Creates a quorum requiring at least one signature.
        pub fn new(required_signatures: u32) -> Result<Self> {
            if required_signatures == 0 {
                return Err(SpecError::Json(
                    "manual authorization quorum must require at least one signature".to_owned(),
                ));
            }
            Ok(Self {
                required_signatures,
            })
        }

        /// Returns the number of required operator signatures.
        pub const fn required_signatures(&self) -> u32 {
            self.required_signatures
        }

        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "kind": "threshold",
                "required_signatures": self.required_signatures,
            })
        }
    }

    /// Constructor parts for [`TypedExecutionSpec`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TypedExecutionSpecParts {
        /// Authoring provenance.
        pub authoring: AuthoringProvenance,
        /// Certified run-level saga policy.
        pub saga: SagaPolicySpec,
        /// Persisted scopes.
        pub scopes: Vec<ScopeSpec>,
        /// Declared seed cells.
        pub seeds: Vec<SeedSpec>,
        /// Descriptor identities required by certification.
        pub descriptor_identities: Vec<DescriptorIdentity>,
        /// Config artifacts referenced by nodes and lineage.
        pub config_refs: Vec<ConfigRef>,
        /// State and framework nodes.
        pub nodes: Vec<NodeSpec>,
        /// Remediation nodes keyed by the forward side-effect node they compensate.
        pub remediations: BTreeMap<NodeId, NodeSpec>,
        /// Planned cells.
        pub cells: Vec<CellSpec>,
        /// Hash-defining value lineage records referenced by cells and inputs.
        pub value_lineages: Vec<ValueLineage>,
        /// Registry-mediated planning lineage frames.
        pub planning_lineage: Vec<OperationLineageFrameSpec>,
        /// Public output contract.
        pub public_outputs: PublicOutputSpec,
    }

    /// Non-semantic audit metadata carried beside a certified spec.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct TypedExecutionSpecAudit {
        /// Descriptor audit references.
        pub descriptor_audit_refs: Vec<DescriptorAuditRef>,
        /// Source package references.
        pub source_package_refs: Vec<SourcePackageRef>,
        /// Non-semantic provenance references.
        pub non_semantic_provenance: Vec<AuditProvenanceRef>,
    }

    /// Descriptor audit reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DescriptorAuditRef {
        /// Descriptor id.
        pub descriptor_id: DescriptorId,
        /// Audit artifact id.
        pub artifact_id: ArtifactId,
        /// Audit artifact digest.
        pub digest: ContentDigest,
    }

    /// Source package reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SourcePackageRef {
        /// Source package name.
        pub name: String,
        /// Source package version.
        pub version: String,
        /// Optional source artifact id.
        pub artifact_id: Option<ArtifactId>,
    }

    /// Non-semantic audit provenance reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AuditProvenanceRef {
        /// Stable provenance kind.
        pub kind: String,
        /// Provenance artifact id.
        pub artifact_id: ArtifactId,
        /// Provenance digest.
        pub digest: ContentDigest,
    }

    /// Authoring provenance for a typed spec.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum AuthoringProvenance {
        /// Spec was authored by expanding one operation.
        OperationExpansion {
            /// Operation descriptor id.
            operation_descriptor_id: DescriptorId,
            /// Operation config digest.
            config_hash: ContentDigest,
        },
        /// Spec was authored by direct state composition.
        StateComposition {
            /// Composition descriptor.
            descriptor: CompositionDescriptor,
            /// Composition config digest.
            config_hash: ContentDigest,
        },
        /// Spec mixes operation expansion and directly declared states.
        MixedComposition {
            /// Mixed composition descriptor.
            descriptor: CompositionDescriptor,
            /// Composition config digest.
            config_hash: ContentDigest,
        },
    }

    impl AuthoringProvenance {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::OperationExpansion {
                    operation_descriptor_id,
                    config_hash,
                } => serde_json::json!({
                    "config_hash": config_hash.as_str(),
                    "kind": "operation_expansion",
                    "operation_descriptor_id": operation_descriptor_id.as_str(),
                }),
                Self::StateComposition {
                    descriptor,
                    config_hash,
                } => serde_json::json!({
                    "config_hash": config_hash.as_str(),
                    "descriptor": descriptor.json(),
                    "kind": "state_composition",
                }),
                Self::MixedComposition {
                    descriptor,
                    config_hash,
                } => serde_json::json!({
                    "config_hash": config_hash.as_str(),
                    "descriptor": descriptor.json(),
                    "kind": "mixed_composition",
                }),
            }
        }
    }

    /// Stable composition descriptor.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompositionDescriptor {
        /// Composition descriptor id.
        pub descriptor_id: DescriptorId,
        /// Stable descriptor name.
        pub name: String,
        /// Stable descriptor version.
        pub version: String,
    }

    impl CompositionDescriptor {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "descriptor_id": self.descriptor_id.as_str(),
                "name": self.name,
                "version": self.version,
            })
        }
    }

    /// Config artifact reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ConfigRef {
        /// Config schema id.
        pub schema_id: SchemaId,
        /// Config artifact id.
        pub artifact_id: ArtifactId,
        /// Canonical config digest.
        pub digest: ContentDigest,
        /// Canonical byte length.
        pub byte_len: u64,
        /// Config artifact media type.
        pub media_type: MediaType,
    }

    impl ConfigRef {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "artifact_id": self.artifact_id.as_str(),
                "byte_len": self.byte_len,
                "digest": self.digest.as_str(),
                "media_type": self.media_type.as_str(),
                "schema_id": self.schema_id.as_str(),
            })
        }
    }

    /// Persisted typed scope.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ScopeSpec {
        /// Scope id.
        pub scope_id: ScopeId,
        /// Parent scope id, when nested.
        pub parent_scope_id: Option<ScopeId>,
        /// Stable author key.
        pub stable_key: StableAuthorKey,
        /// Operation lineage active when this scope id was derived.
        pub planning_lineage: PlanningLineage,
    }

    impl ScopeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "parent_scope_id": self.parent_scope_id.as_ref().map(ScopeId::as_str),
                "planning_lineage": self.planning_lineage.json(),
                "scope_id": self.scope_id.as_str(),
                "stable_key": self.stable_key.as_str(),
            })
        }
    }

    /// Declared root seed cell.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SeedSpec {
        /// Seed id.
        pub seed_id: SeedId,
        /// Stable seed key.
        pub seed_key: StableAuthorKey,
        /// Planned seed cell id.
        pub cell_id: CellId,
        /// Owning scope id.
        pub scope_id: ScopeId,
        /// Seed semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Seed schema id.
        pub schema_id: SchemaId,
        /// Required launch digest, if fixed by the spec.
        pub required_digest: Option<ContentDigest>,
    }

    impl SeedSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "cell_id": self.cell_id.as_str(),
                "required_digest": self.required_digest.as_ref().map(ContentDigest::as_str),
                "schema_id": self.schema_id.as_str(),
                "scope_id": self.scope_id.as_str(),
                "seed_id": self.seed_id.as_str(),
                "seed_key": self.seed_key.as_str(),
                "semantic_type_id": self.semantic_type_id.as_str(),
            })
        }
    }

    /// Descriptor identities embedded in the hash-defining spec.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum DescriptorIdentity {
        /// State descriptor identity.
        State(Box<StateDescriptorIdentity>),
        /// Operation descriptor identity.
        Operation(Box<OperationDescriptorIdentity>),
        /// Public output renderer descriptor identity.
        Renderer(Box<RendererDescriptorIdentity>),
    }

    impl DescriptorIdentity {
        /// Returns this descriptor's family.
        pub fn family(&self) -> DescriptorFamily {
            match self {
                Self::State(_) => DescriptorFamily::State,
                Self::Operation(_) => DescriptorFamily::Operation,
                Self::Renderer(_) => DescriptorFamily::Renderer,
            }
        }

        /// Returns this descriptor's content-addressed id.
        pub fn descriptor_id(&self) -> &DescriptorId {
            match self {
                Self::State(identity) => &identity.descriptor_id,
                Self::Operation(identity) => &identity.descriptor_id,
                Self::Renderer(identity) => &identity.descriptor_id,
            }
        }

        /// Computes the canonical descriptor payload digest.
        pub fn descriptor_digest(&self) -> Result<ContentDigest> {
            content_digest(self.json())
        }

        /// Computes the descriptor reference used by node and contract records.
        pub fn descriptor_ref(&self) -> Result<DescriptorRef> {
            Ok(DescriptorRef {
                family: self.family(),
                descriptor_id: self.descriptor_id().clone(),
                descriptor_digest: self.descriptor_digest()?,
            })
        }

        fn json(&self) -> serde_json::Value {
            match self {
                Self::State(identity) => {
                    let mut json = identity.json();
                    json["descriptor_family"] = serde_json::json!("state");
                    json
                }
                Self::Operation(identity) => {
                    let mut json = identity.json();
                    json["descriptor_family"] = serde_json::json!("operation");
                    json
                }
                Self::Renderer(identity) => {
                    let mut json = identity.json();
                    json["descriptor_family"] = serde_json::json!("renderer");
                    json
                }
            }
        }
    }

    /// Descriptor family for content-addressed descriptor references.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum DescriptorFamily {
        /// State descriptor reference.
        State,
        /// Operation descriptor reference.
        Operation,
        /// Public-output renderer descriptor reference.
        Renderer,
    }

    impl DescriptorFamily {
        /// Returns the persisted descriptor family string.
        pub fn as_str(self) -> &'static str {
            match self {
                Self::State => "state",
                Self::Operation => "operation",
                Self::Renderer => "renderer",
            }
        }

        /// Parses a persisted descriptor family string.
        pub fn parse(value: &str) -> Result<Self> {
            match value {
                "state" => Ok(Self::State),
                "operation" => Ok(Self::Operation),
                "renderer" => Ok(Self::Renderer),
                other => Err(json_error(format!(
                    "unsupported descriptor family {other:?}"
                ))),
            }
        }
    }

    /// Content-addressed reference to a descriptor table entry.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct DescriptorRef {
        /// Descriptor family.
        pub family: DescriptorFamily,
        /// Descriptor identity.
        pub descriptor_id: DescriptorId,
        /// Canonical digest of the descriptor identity payload.
        pub descriptor_digest: ContentDigest,
    }

    impl DescriptorRef {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "descriptor_digest": self.descriptor_digest.as_str(),
                "descriptor_family": self.family.as_str(),
                "descriptor_id": self.descriptor_id.as_str(),
            })
        }
    }

    struct DescriptorJsonIndex {
        refs: BTreeMap<String, DescriptorRef>,
    }

    impl DescriptorJsonIndex {
        fn new(descriptors: &[DescriptorIdentity]) -> Result<Self> {
            let mut refs = BTreeMap::new();
            for descriptor in descriptors {
                let reference = descriptor.descriptor_ref()?;
                let key = reference.descriptor_id.as_str().to_owned();
                if refs.insert(key.clone(), reference).is_some() {
                    return Err(json_error(format!("duplicate descriptor identity {key}")));
                }
            }
            Ok(Self { refs })
        }

        fn require(
            &self,
            descriptor_id: &DescriptorId,
            family: DescriptorFamily,
        ) -> Result<&DescriptorRef> {
            let reference = self.refs.get(descriptor_id.as_str()).ok_or_else(|| {
                json_error(format!("missing descriptor identity {descriptor_id}"))
            })?;
            if reference.family != family {
                return Err(json_error(format!(
                    "descriptor {descriptor_id} is {}, expected {}",
                    reference.family.as_str(),
                    family.as_str()
                )));
            }
            Ok(reference)
        }
    }

    /// State descriptor identity.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StateDescriptorIdentity {
        /// State descriptor id.
        pub descriptor_id: DescriptorId,
        /// Stable state descriptor name.
        pub name: String,
        /// State kind.
        pub state_kind: StateKind,
        /// State version.
        pub state_version: StateVersion,
        /// Config schema id.
        pub config_schema_id: SchemaId,
        /// Input schema id.
        pub input_schema_id: SchemaId,
        /// Output schema id.
        pub output_schema_id: SchemaId,
        /// Output semantic type id.
        pub output_semantic_type_id: SemanticTypeId,
        /// Effect kind.
        pub effect_kind: EffectKind,
        /// Semantic effect class string.
        pub effect_class: String,
        /// Framework-owned effect descriptor name.
        pub effect_name: String,
        /// Effect descriptor version.
        pub effect_version: EffectVersion,
        /// Capability descriptor set.
        pub capabilities: CapabilitySetDescriptor,
        /// Runner kind recorded by the registered state.
        pub runner: String,
        /// Side-effect contract digest for external mutations.
        pub side_effect_contract_digest: Option<ContentDigest>,
    }

    impl StateDescriptorIdentity {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "capabilities": capability_set_json(&self.capabilities),
                "config_schema_id": self.config_schema_id.as_str(),
                "descriptor_id": self.descriptor_id.as_str(),
                "effect_class": self.effect_class.as_str(),
                "effect_kind": self.effect_kind.as_str(),
                "effect_name": self.effect_name.as_str(),
                "effect_version": self.effect_version.as_str(),
                "input_schema_id": self.input_schema_id.as_str(),
                "name": self.name.as_str(),
                "output_schema_id": self.output_schema_id.as_str(),
                "output_semantic_type_id": self.output_semantic_type_id.as_str(),
                "runner": self.runner.as_str(),
                "side_effect_contract_digest": self.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
                "state_kind": self.state_kind.as_str(),
                "state_version": self.state_version.as_str(),
            })
        }
    }

    /// Operation descriptor identity.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OperationDescriptorIdentity {
        /// Operation descriptor id.
        pub descriptor_id: DescriptorId,
        /// Stable operation descriptor name.
        pub name: String,
        /// Operation kind.
        pub operation_kind: OperationKind,
        /// Operation version.
        pub operation_version: OperationVersion,
        /// Config schema id.
        pub config_schema_id: SchemaId,
        /// Input schema id.
        pub input_schema_id: SchemaId,
        /// Output schema id.
        pub output_schema_id: SchemaId,
        /// Deterministic expansion ABI.
        pub expansion_abi: String,
    }

    impl OperationDescriptorIdentity {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "config_schema_id": self.config_schema_id.as_str(),
                "descriptor_id": self.descriptor_id.as_str(),
                "expansion_abi": self.expansion_abi.as_str(),
                "input_schema_id": self.input_schema_id.as_str(),
                "name": self.name.as_str(),
                "operation_kind": self.operation_kind.as_str(),
                "operation_version": self.operation_version.as_str(),
                "output_schema_id": self.output_schema_id.as_str(),
            })
        }
    }

    /// Renderer descriptor identity.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RendererDescriptorIdentity {
        /// Renderer descriptor id.
        pub descriptor_id: DescriptorId,
        /// Renderer kind.
        pub renderer_kind: RendererKind,
        /// Renderer version.
        pub renderer_version: RendererVersion,
        /// Public output schema id.
        pub public_schema_id: SchemaId,
        /// Canonicalizer identity.
        pub canonicalizer_identity: CanonicalizerIdentity,
    }

    impl RendererDescriptorIdentity {
        /// Computes the content-addressed renderer descriptor id from descriptor fields.
        pub fn expected_descriptor_id(&self) -> Result<DescriptorId> {
            descriptor_id_from_payload(serde_json::json!({
                "canonicalizer_identity": self.canonicalizer_identity.as_str(),
                "public_schema_id": self.public_schema_id.as_str(),
                "renderer_kind": self.renderer_kind.as_str(),
                "renderer_version": self.renderer_version.as_str(),
            }))
        }

        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "canonicalizer_identity": self.canonicalizer_identity.as_str(),
                "descriptor_id": self.descriptor_id.as_str(),
                "public_schema_id": self.public_schema_id.as_str(),
                "renderer_kind": self.renderer_kind.as_str(),
                "renderer_version": self.renderer_version.as_str(),
            })
        }
    }

    fn descriptor_id_from_payload(value: serde_json::Value) -> Result<DescriptorId> {
        Ok(DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *content_digest(value)?.digest(),
        ))
    }

    /// Typed node spec for state and framework nodes.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NodeSpec {
        /// Node id.
        pub node_id: NodeId,
        /// Stable node key.
        pub stable_key: StableAuthorKey,
        /// Owning scope id.
        pub scope_id: ScopeId,
        /// State kind or framework state kind.
        pub state_kind: StateKind,
        /// State version or framework state version.
        pub state_version: StateVersion,
        /// Descriptor id.
        pub descriptor_id: DescriptorId,
        /// Config reference.
        pub config_ref: ConfigRef,
        /// Input bindings.
        pub input_bindings: InputBindingSpec,
        /// Output cell id.
        pub output_cell: CellId,
        /// Effect kind.
        pub effect_kind: EffectKind,
        /// Capability bindings.
        pub capability_bindings: CapabilitySetDescriptor,
        /// Adapter bindings.
        pub adapter_bindings: Vec<AdapterBinding>,
        /// Side-effect contract, when applicable.
        pub side_effect: Option<SideEffectContractSpec>,
        /// Framework node metadata, when framework-owned.
        pub framework: Option<FrameworkNodeSpec>,
        /// Planning lineage active for this node.
        pub planning_lineage: PlanningLineage,
        /// Deterministic predecessor node ids.
        pub deterministic_predecessors: Vec<NodeId>,
    }

    impl NodeSpec {
        fn json(&self, descriptors: &DescriptorJsonIndex) -> Result<serde_json::Value> {
            let descriptor_ref =
                descriptors.require(&self.descriptor_id, DescriptorFamily::State)?;
            Ok(serde_json::json!({
                "adapter_bindings": self.adapter_bindings.iter().map(AdapterBinding::json).collect::<Vec<_>>(),
                "config_ref": self.config_ref.json(),
                "descriptor_ref": descriptor_ref.json(),
                "deterministic_predecessors": self.deterministic_predecessors
                    .iter()
                    .map(NodeId::as_str)
                    .collect::<Vec<_>>(),
                "framework": self.framework.as_ref().map(FrameworkNodeSpec::json).transpose()?,
                "input_bindings": self.input_bindings.json(),
                "node_id": self.node_id.as_str(),
                "output_cell": self.output_cell.as_str(),
                "planning_lineage": self.planning_lineage.json(),
                "scope_id": self.scope_id.as_str(),
                "side_effect": self.side_effect.as_ref().map(SideEffectContractSpec::json),
                "stable_key": self.stable_key.as_str(),
            }))
        }
    }

    /// Adapter binding persisted for behaviorally relevant adapters/connectors.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AdapterBinding {
        /// Adapter kind.
        pub adapter_kind: AdapterKind,
        /// Adapter version.
        pub adapter_version: AdapterVersion,
        /// Optional binding digest.
        pub binding_digest: Option<ContentDigest>,
    }

    impl AdapterBinding {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "adapter_kind": self.adapter_kind.as_str(),
                "adapter_version": self.adapter_version.as_str(),
                "binding_digest": self.binding_digest.as_ref().map(ContentDigest::as_str),
            })
        }
    }

    /// Resource claim declared by a side-effect node.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ResourceClaimSpec {
        /// The adapter records a concrete exclusive key before crossing the uncertainty boundary.
        Exclusive {
            /// Resource namespace for the exclusive lane.
            namespace: ResourceNamespace,
            /// Schema id for the adapter-recorded key evidence.
            key_schema: SchemaId,
        },
        /// The adapter records the exact touched key set as typed post-execution evidence.
        ExactTouchedSet {
            /// Resource namespace for the touched set evidence.
            namespace: ResourceNamespace,
            /// Schema id for the touched set evidence.
            evidence_schema: SchemaId,
        },
        /// No framework-derived cross-run concurrency claim is made.
        ManualOnly,
    }

    impl ResourceClaimSpec {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::Exclusive {
                    namespace,
                    key_schema,
                } => serde_json::json!({
                    "kind": "exclusive",
                    "exclusive": {
                        "key_schema": key_schema.as_str(),
                        "namespace": namespace.as_str(),
                    },
                }),
                Self::ExactTouchedSet {
                    namespace,
                    evidence_schema,
                } => serde_json::json!({
                    "kind": "exact_touched_set",
                    "exact_touched_set": {
                        "evidence_schema": evidence_schema.as_str(),
                        "namespace": namespace.as_str(),
                    },
                }),
                Self::ManualOnly => serde_json::json!({
                    "kind": "manual_only",
                }),
            }
        }
    }

    /// Certified side-effect terminal verification policy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SideEffectVerificationSpec {
        /// Receipt evidence is terminal. This is final-at-risk if the external chain can reorg.
        Receipt,
        /// Confirmation/finality evidence at a certified positive depth is terminal.
        Finalized {
            /// Required confirmation/finality depth.
            depth: u64,
        },
    }

    impl SideEffectVerificationSpec {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::Receipt => serde_json::json!({
                    "kind": "receipt",
                }),
                Self::Finalized { depth } => serde_json::json!({
                    "finalized": {
                        "depth": depth,
                    },
                    "kind": "finalized",
                }),
            }
        }
    }

    /// Side-effect contract persisted in node specs.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectContractSpec {
        /// Side-effect contract digest.
        pub contract_digest: ContentDigest,
        /// Mandatory cross-run resource claim declaration.
        pub resource_claim: ResourceClaimSpec,
        /// Certified terminal verification policy.
        pub verification: SideEffectVerificationSpec,
    }

    impl SideEffectContractSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "contract_digest": self.contract_digest.as_str(),
                "resource_claim": self.resource_claim.json(),
                "verification": self.verification.json(),
            })
        }
    }

    /// Framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FrameworkNodeSpec {
        /// Same-value bridge framework node.
        Bridge(BridgeNodeSpec),
        /// Side-effect verification framework node.
        SideEffectVerify(SideEffectVerifyNodeSpec),
        /// Public-output render framework node.
        PublicOutputRender(PublicOutputRenderNodeSpec),
        /// Retention-manifest projection lifecycle framework node.
        ProjectRetentionManifest(ProjectRetentionManifestNodeSpec),
        /// Complete-run lifecycle framework node.
        CompleteRun(CompleteRunNodeSpec),
        /// Saga-terminal resolution lifecycle framework node.
        ResolveSagaTerminal(ResolveSagaTerminalNodeSpec),
    }

    impl FrameworkNodeSpec {
        /// Returns the deterministic framework config kind persisted for this node.
        pub fn config_kind(&self) -> &'static str {
            match self {
                Self::Bridge(_) => "bridge_same_value",
                Self::SideEffectVerify(_) => "side_effect_verify",
                Self::PublicOutputRender(_) => "public_output_render",
                Self::ProjectRetentionManifest(_) => "project_retention_manifest",
                Self::CompleteRun(_) => "complete_run",
                Self::ResolveSagaTerminal(_) => "resolve_saga_terminal",
            }
        }

        fn json(&self) -> Result<serde_json::Value> {
            Ok(match self {
                Self::Bridge(spec) => serde_json::json!({
                    "bridge": spec.json(),
                    "kind": "bridge",
                }),
                Self::SideEffectVerify(spec) => serde_json::json!({
                    "kind": "side_effect_verify",
                    "side_effect_verify": spec.json(),
                }),
                Self::PublicOutputRender(spec) => serde_json::json!({
                    "kind": "public_output_render",
                    "public_output_render": spec.json()?,
                }),
                Self::ProjectRetentionManifest(spec) => serde_json::json!({
                    "kind": "project_retention_manifest",
                    "project_retention_manifest": spec.json(),
                }),
                Self::CompleteRun(spec) => serde_json::json!({
                    "complete_run": spec.json(),
                    "kind": "complete_run",
                }),
                Self::ResolveSagaTerminal(spec) => serde_json::json!({
                    "kind": "resolve_saga_terminal",
                    "resolve_saga_terminal": spec.json(),
                }),
            })
        }
    }

    /// Side-effect verification framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectVerifyNodeSpec {
        /// Certified side-effect pair id.
        pub pair_id: SideEffectPairId,
        /// Submit node that owns the external mutation boundary.
        pub submit_node_id: NodeId,
    }

    impl SideEffectVerifyNodeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "pair_id": self.pair_id.as_str(),
                "submit_node_id": self.submit_node_id.as_str(),
            })
        }
    }

    /// Resolved side-effect submit/verify pair authority derived from a typed spec.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SideEffectVerifyPairRef<'a> {
        /// Certified side-effect pair id.
        pub pair_id: &'a SideEffectPairId,
        /// Framework verify node.
        pub verify_node: &'a NodeSpec,
        /// Framework verify metadata.
        pub verify: &'a SideEffectVerifyNodeSpec,
        /// Submit node that owns the external mutation boundary.
        pub submit_node: &'a NodeSpec,
        /// Submit node side-effect contract.
        pub submit_contract: &'a SideEffectContractSpec,
        /// Submit node internal output cell used as the structural anchor.
        pub submit_output_cell: &'a CellId,
    }

    /// Resolves a side-effect verify pair from typed spec node collections.
    pub fn resolve_side_effect_verify_pair<'a>(
        nodes: &'a [NodeSpec],
        remediations: &'a BTreeMap<NodeId, NodeSpec>,
        verify_node: &'a NodeSpec,
    ) -> Result<SideEffectVerifyPairRef<'a>> {
        let Some(FrameworkNodeSpec::SideEffectVerify(verify)) = &verify_node.framework else {
            return Err(side_effect_verify_pair_error(
                SideEffectVerifyPairErrorKind::NotVerifyNode,
                format!(
                    "node {} is not a side-effect verify node",
                    verify_node.node_id
                ),
            ));
        };
        let submit_node = nodes
            .iter()
            .chain(remediations.values())
            .find(|node| node.node_id == verify.submit_node_id)
            .ok_or_else(|| {
                side_effect_verify_pair_error(
                    SideEffectVerifyPairErrorKind::MissingSubmitNode,
                    format!(
                        "side-effect verify node {} references missing submit node {}",
                        verify_node.node_id, verify.submit_node_id
                    ),
                )
            })?;
        if submit_node.framework.is_some() {
            return Err(side_effect_verify_pair_error(
                SideEffectVerifyPairErrorKind::FrameworkSubmitNode,
                format!(
                    "side-effect verify node {} references framework submit node {}",
                    verify_node.node_id, submit_node.node_id
                ),
            ));
        }
        let submit_contract = submit_node.side_effect.as_ref().ok_or_else(|| {
            side_effect_verify_pair_error(
                SideEffectVerifyPairErrorKind::NonSideEffectSubmitNode,
                format!(
                    "side-effect verify node {} references non-side-effect submit node {}",
                    verify_node.node_id, submit_node.node_id
                ),
            )
        })?;
        let expected_pair = side_effect_pair_id(
            &submit_node.node_id,
            &submit_node.output_cell,
            submit_contract,
        )?;
        if verify.pair_id != expected_pair {
            return Err(side_effect_verify_pair_error(
                SideEffectVerifyPairErrorKind::PairIdMismatch,
                format!(
                    "side-effect verify node {} pair id is not stable-id derived",
                    verify_node.node_id
                ),
            ));
        }
        Ok(SideEffectVerifyPairRef {
            pair_id: &verify.pair_id,
            verify_node,
            verify,
            submit_node,
            submit_contract,
            submit_output_cell: &submit_node.output_cell,
        })
    }

    fn side_effect_verify_pair_error(
        kind: SideEffectVerifyPairErrorKind,
        message: String,
    ) -> SpecError {
        SpecError::SideEffectVerifyPair { kind, message }
    }

    /// Same-value bridge framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeNodeSpec {
        /// Bridge direction.
        pub bridge_kind: BridgeKind,
        /// Source scope id.
        pub source_scope_id: ScopeId,
        /// Target scope id.
        pub target_scope_id: ScopeId,
        /// Source cell id.
        pub source_cell_id: CellId,
        /// Target cell id.
        pub target_cell_id: CellId,
        /// Bridge semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Bridge schema id.
        pub schema_id: SchemaId,
        /// Bridge policy.
        pub policy: BridgePolicy,
        /// Bridge provenance.
        pub provenance: BridgeProvenance,
    }

    impl BridgeNodeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "bridge_kind": self.bridge_kind.as_str(),
                "policy": self.policy.as_str(),
                "provenance": self.provenance.as_str(),
                "schema_id": self.schema_id.as_str(),
                "semantic_type_id": self.semantic_type_id.as_str(),
                "source_cell_id": self.source_cell_id.as_str(),
                "source_scope_id": self.source_scope_id.as_str(),
                "target_cell_id": self.target_cell_id.as_str(),
                "target_scope_id": self.target_scope_id.as_str(),
            })
        }
    }

    /// Bridge direction.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum BridgeKind {
        /// Parent value imported into a child scope.
        ImportFromParent,
        /// Child value exported into its parent scope.
        ExportToParent,
    }

    impl BridgeKind {
        fn as_str(self) -> &'static str {
            match self {
                Self::ImportFromParent => "import_from_parent",
                Self::ExportToParent => "export_to_parent",
            }
        }
    }

    /// Bridge policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum BridgePolicy {
        /// Same-run same-value bridge.
        SameRunSameValue,
    }

    impl BridgePolicy {
        fn as_str(self) -> &'static str {
            match self {
                Self::SameRunSameValue => "same_run_same_value",
            }
        }
    }

    /// Framework bridge provenance.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum BridgeProvenance {
        /// Framework child-scope bridge v1.
        FrameworkChildScopeV1,
    }

    impl BridgeProvenance {
        fn as_str(self) -> &'static str {
            match self {
                Self::FrameworkChildScopeV1 => "framework_child_scope_v1",
            }
        }
    }

    /// Public output render framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PublicOutputRenderNodeSpec {
        /// Public output schema id.
        pub public_schema_id: SchemaId,
        /// Digest of the public output spec.
        pub output_spec_digest: ContentDigest,
        /// Renderer descriptor.
        pub renderer_descriptor: RendererDescriptorIdentity,
        /// Required public cells.
        pub required_cells: Vec<PublicOutputCell>,
    }

    impl PublicOutputRenderNodeSpec {
        fn json(&self) -> Result<serde_json::Value> {
            let renderer_ref =
                DescriptorIdentity::Renderer(Box::new(self.renderer_descriptor.clone()))
                    .descriptor_ref()?;
            Ok(serde_json::json!({
                "output_spec_digest": self.output_spec_digest.as_str(),
                "public_schema_id": self.public_schema_id.as_str(),
                "renderer_descriptor_ref": renderer_ref.json(),
                "required_cells": self.required_cells.iter().map(PublicOutputCell::json).collect::<Vec<_>>(),
            }))
        }
    }

    /// Retention-manifest projection lifecycle framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProjectRetentionManifestNodeSpec {
        /// Public output schema whose retained stream evidence is projected.
        pub public_schema_id: SchemaId,
        /// Public-output render receipt cell that orders this projection after rendering.
        pub public_output_receipt_cell: CellId,
    }

    impl ProjectRetentionManifestNodeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "public_output_receipt_cell": self.public_output_receipt_cell.as_str(),
                "public_schema_id": self.public_schema_id.as_str(),
            })
        }
    }

    /// Complete-run lifecycle framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompleteRunNodeSpec {
        /// Public output schema whose completion evidence terminates the run.
        pub public_schema_id: SchemaId,
        /// Retention-manifest projection receipt cell that orders completion after retention.
        pub retention_manifest_receipt_cell: CellId,
    }

    impl CompleteRunNodeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "public_schema_id": self.public_schema_id.as_str(),
                "retention_manifest_receipt_cell": self.retention_manifest_receipt_cell.as_str(),
            })
        }
    }

    /// Resolve-saga-terminal lifecycle framework node metadata.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ResolveSagaTerminalNodeSpec {
        /// Public output schema whose terminal status is resolved.
        pub public_schema_id: SchemaId,
    }

    impl ResolveSagaTerminalNodeSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "public_schema_id": self.public_schema_id.as_str(),
            })
        }
    }

    /// State input binding spec.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct InputBindingSpec {
        /// Input schema id.
        pub input_schema_id: SchemaId,
        /// Input descriptor id.
        pub input_descriptor_id: DescriptorId,
        /// Root input binding node.
        pub root: InputBindingNodeSpec,
        /// Canonical binding digest.
        pub digest: ContentDigest,
    }

    impl InputBindingSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "digest": self.digest.as_str(),
                "input_descriptor_id": self.input_descriptor_id.as_str(),
                "input_schema_id": self.input_schema_id.as_str(),
                "root": self.root.json(),
            })
        }
    }

    /// Input binding tree node.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum InputBindingNodeSpec {
        /// Unit input.
        Unit,
        /// Typed cell input.
        Cell(Box<InputBindingCellSpec>),
        /// Tuple input.
        Tuple(Vec<InputBindingNodeSpec>),
        /// Struct input.
        Struct(Vec<NamedInputBindingSpec>),
        /// Vector input.
        Vec {
            /// Elements.
            elements: Vec<InputBindingNodeSpec>,
            /// Ordering evidence.
            ordering: OrderingEvidence,
            /// Stable domain key refs.
            domain_keys: Vec<StableDomainKeyRef>,
        },
        /// Non-empty vector input.
        NonEmptyVec {
            /// Elements.
            elements: Vec<InputBindingNodeSpec>,
            /// Ordering evidence.
            ordering: OrderingEvidence,
            /// Stable domain key refs.
            domain_keys: Vec<StableDomainKeyRef>,
        },
    }

    impl InputBindingNodeSpec {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::Unit => serde_json::json!({ "kind": "unit" }),
                Self::Cell(cell) => {
                    let mut json = cell.json();
                    json["kind"] = serde_json::json!("cell");
                    json
                }
                Self::Tuple(elements) => serde_json::json!({
                    "elements": elements.iter().map(InputBindingNodeSpec::json).collect::<Vec<_>>(),
                    "kind": "tuple",
                }),
                Self::Struct(fields) => serde_json::json!({
                    "fields": fields.iter().map(NamedInputBindingSpec::json).collect::<Vec<_>>(),
                    "kind": "struct",
                }),
                Self::Vec {
                    elements,
                    ordering,
                    domain_keys,
                } => serde_json::json!({
                    "domain_keys": domain_keys.iter().map(StableDomainKeyRef::json).collect::<Vec<_>>(),
                    "elements": elements.iter().map(InputBindingNodeSpec::json).collect::<Vec<_>>(),
                    "kind": "vec",
                    "ordering": ordering.as_str(),
                }),
                Self::NonEmptyVec {
                    elements,
                    ordering,
                    domain_keys,
                } => serde_json::json!({
                    "domain_keys": domain_keys.iter().map(StableDomainKeyRef::json).collect::<Vec<_>>(),
                    "elements": elements.iter().map(InputBindingNodeSpec::json).collect::<Vec<_>>(),
                    "kind": "non_empty_vec",
                    "ordering": ordering.as_str(),
                }),
            }
        }
    }

    /// Typed input binding cell leaf.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct InputBindingCellSpec {
        /// Input field path.
        pub field_path: PublicFieldPath,
        /// Cell id.
        pub cell_id: CellId,
        /// Semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Schema id.
        pub schema_id: SchemaId,
        /// Required terminal policy.
        pub required_terminal: RequiredTerminal,
        /// Value lineage ref.
        pub value_lineage: ValueLineageRef,
    }

    impl InputBindingCellSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "cell_id": self.cell_id.as_str(),
                "field_path": self.field_path.as_str(),
                "required_terminal": self.required_terminal.as_str(),
                "schema_id": self.schema_id.as_str(),
                "semantic_type_id": self.semantic_type_id.as_str(),
                "value_lineage": self.value_lineage.json(),
            })
        }
    }

    /// Named struct input field.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NamedInputBindingSpec {
        /// Field path.
        pub field_path: PublicFieldPath,
        /// Field node.
        pub node: InputBindingNodeSpec,
    }

    impl NamedInputBindingSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "field_path": self.field_path.as_str(),
                "node": self.node.json(),
            })
        }
    }

    /// Ordering evidence for dynamic collections.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum OrderingEvidence {
        /// Author-provided vector order.
        ExplicitAuthorOrder,
        /// Canonical stable-domain-key order.
        StableDomainKey,
    }

    impl OrderingEvidence {
        fn as_str(self) -> &'static str {
            match self {
                Self::ExplicitAuthorOrder => "explicit_author_order",
                Self::StableDomainKey => "stable_domain_key",
            }
        }
    }

    /// Stable domain key reference.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct StableDomainKeyRef {
        /// Domain key schema id.
        pub schema_id: SchemaId,
        /// Domain key content digest.
        pub content_digest: ContentDigest,
    }

    impl StableDomainKeyRef {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "content_digest": self.content_digest.as_str(),
                "schema_id": self.schema_id.as_str(),
            })
        }
    }

    /// Planned cell producer.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum CellProducer {
        /// Cell produced by a node.
        Node(NodeId),
        /// Cell produced by a seed.
        Seed(SeedId),
    }

    impl CellProducer {
        fn json(&self) -> serde_json::Value {
            match self {
                Self::Node(node_id) => serde_json::json!({
                    "kind": "node",
                    "node_id": node_id.as_str(),
                }),
                Self::Seed(seed_id) => serde_json::json!({
                    "kind": "seed",
                    "seed_id": seed_id.as_str(),
                }),
            }
        }
    }

    /// Planned typed cell.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CellSpec {
        /// Cell id.
        pub cell_id: CellId,
        /// Cell producer.
        pub producer: CellProducer,
        /// Owning scope id.
        pub scope_id: ScopeId,
        /// Semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Schema id.
        pub schema_id: SchemaId,
        /// Value lineage ref.
        pub value_lineage: ValueLineageRef,
        /// Terminal policy.
        pub terminal_policy: CellTerminalPolicy,
        /// Storage policy.
        pub storage_policy: StoragePolicy,
        /// Redaction policy.
        pub redaction_policy: RedactionPolicy,
    }

    impl CellSpec {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "cell_id": self.cell_id.as_str(),
                "producer": self.producer.json(),
                "redaction_policy": self.redaction_policy.as_str(),
                "schema_id": self.schema_id.as_str(),
                "scope_id": self.scope_id.as_str(),
                "semantic_type_id": self.semantic_type_id.as_str(),
                "storage_policy": self.storage_policy.as_str(),
                "terminal_policy": self.terminal_policy.as_str(),
                "value_lineage": self.value_lineage.json(),
            })
        }
    }

    /// Cell terminal policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum CellTerminalPolicy {
        /// Cell must be produced.
        ProducedOnly,
        /// Cell may be skipped with typed skip evidence.
        MaybeSkipped,
    }

    impl CellTerminalPolicy {
        fn as_str(self) -> &'static str {
            match self {
                Self::ProducedOnly => "produced_only",
                Self::MaybeSkipped => "maybe_skipped",
            }
        }
    }

    /// Cell storage policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum StoragePolicy {
        /// Store canonical value bytes by content address.
        ContentAddressed,
        /// Store artifact reference by content address.
        ArtifactReference,
        /// Store public output render artifact by content address.
        PublicOutputArtifact,
    }

    impl StoragePolicy {
        fn as_str(self) -> &'static str {
            match self {
                Self::ContentAddressed => "content_addressed",
                Self::ArtifactReference => "artifact_reference",
                Self::PublicOutputArtifact => "public_output_artifact",
            }
        }
    }

    /// Cell redaction policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum RedactionPolicy {
        /// Cell may appear on public persisted surfaces.
        Public,
        /// Cell content must be redacted from public persisted surfaces.
        Redacted,
    }

    impl RedactionPolicy {
        fn as_str(self) -> &'static str {
            match self {
                Self::Public => "public",
                Self::Redacted => "redacted",
            }
        }
    }

    /// Value-lineage reference.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ValueLineageRef {
        /// Value-lineage digest.
        pub lineage_digest: ContentDigest,
    }

    impl ValueLineageRef {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "lineage_digest": self.lineage_digest.as_str(),
            })
        }
    }

    /// Hash-defining value-lineage evidence for a planned cell.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ValueLineage {
        /// Value-lineage reference digest.
        pub lineage_ref: ValueLineageRef,
        /// Scope containing this value.
        pub scope_id: ScopeId,
        /// Value producer.
        pub producer: CellProducer,
        /// Input cells used to produce this value.
        pub input_cells: Vec<CellId>,
        /// Config reference digest used by the producer, when any.
        pub config_ref_digest: Option<ContentDigest>,
        /// Planning lineage active when this value was produced.
        pub planning_lineage: PlanningLineage,
        /// Stable domain keys associated with this value.
        pub domain_keys: Vec<StableDomainKeyRef>,
        /// Lineage transform policy.
        pub transform_policy: LineageTransformPolicy,
    }

    impl ValueLineage {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "config_ref_digest": self.config_ref_digest.as_ref().map(ContentDigest::as_str),
                "domain_keys": self.domain_keys.iter().map(StableDomainKeyRef::json).collect::<Vec<_>>(),
                "input_cells": self.input_cells.iter().map(CellId::as_str).collect::<Vec<_>>(),
                "lineage_ref": self.lineage_ref.json(),
                "planning_lineage": self.planning_lineage.json(),
                "producer": self.producer.json(),
                "scope_id": self.scope_id.as_str(),
                "transform_policy": self.transform_policy.as_str(),
            })
        }
    }

    /// Value-lineage transform policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum LineageTransformPolicy {
        /// Source value.
        Source,
        /// State output.
        StateOutput,
        /// Same-value bridge.
        SameValueBridge,
    }

    impl LineageTransformPolicy {
        fn as_str(self) -> &'static str {
            match self {
                Self::Source => "source",
                Self::StateOutput => "state_output",
                Self::SameValueBridge => "same_value_bridge",
            }
        }
    }

    /// Planning lineage for a node.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PlanningLineage {
        /// Active operation instances from outermost to innermost.
        pub active_operation_instances: Vec<OperationInstanceId>,
        /// Completed operation frame digests in this scope.
        pub completed_operation_frames: Vec<ContentDigest>,
        /// Digest of the operation lineage sequence.
        pub lineage_digest: ContentDigest,
    }

    impl PlanningLineage {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "active_operation_instances": self.active_operation_instances
                    .iter()
                    .map(OperationInstanceId::as_str)
                    .collect::<Vec<_>>(),
                "completed_operation_frames": self.completed_operation_frames
                    .iter()
                    .map(ContentDigest::as_str)
                    .collect::<Vec<_>>(),
                "lineage_digest": self.lineage_digest.as_str(),
            })
        }
    }

    /// Registry-mediated operation lineage frame.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OperationLineageFrameSpec {
        /// Operation instance id.
        pub operation_instance_id: OperationInstanceId,
        /// Stable operation key.
        pub operation_key: StableAuthorKey,
        /// Owning scope id.
        pub scope_id: ScopeId,
        /// Operation descriptor id.
        pub operation_descriptor_id: DescriptorId,
        /// Config reference digest.
        pub config_ref_digest: ContentDigest,
        /// Typed input binding evidence for the operation expansion.
        pub input_bindings: InputBindingSpec,
        /// Input binding digest.
        pub input_binding_digest: ContentDigest,
        /// Operation lineage active before this operation was expanded.
        pub parent_planning_lineage: PlanningLineage,
        /// Output cells returned by expansion.
        pub output_cells: Vec<CellId>,
        /// Lineage frame digest.
        pub lineage_digest: ContentDigest,
    }

    impl OperationLineageFrameSpec {
        fn json(&self, descriptors: &DescriptorJsonIndex) -> Result<serde_json::Value> {
            let descriptor_ref =
                descriptors.require(&self.operation_descriptor_id, DescriptorFamily::Operation)?;
            Ok(serde_json::json!({
                "config_ref_digest": self.config_ref_digest.as_str(),
                "input_bindings": self.input_bindings.json(),
                "input_binding_digest": self.input_binding_digest.as_str(),
                "lineage_digest": self.lineage_digest.as_str(),
                "operation_descriptor_ref": descriptor_ref.json(),
                "operation_instance_id": self.operation_instance_id.as_str(),
                "operation_key": self.operation_key.as_str(),
                "output_cells": self.output_cells.iter().map(CellId::as_str).collect::<Vec<_>>(),
                "parent_planning_lineage": self.parent_planning_lineage.json(),
                "scope_id": self.scope_id.as_str(),
            }))
        }
    }

    /// Public output contract.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PublicOutputSpec {
        /// Public output schema id.
        pub public_schema_id: SchemaId,
        /// Named output cells.
        pub outputs: Vec<PublicOutputCell>,
        /// Renderer descriptor.
        pub renderer_descriptor: RendererDescriptorIdentity,
    }

    impl PublicOutputSpec {
        /// Computes the canonical digest of this public output spec.
        pub fn digest(&self) -> Result<ContentDigest> {
            content_digest(self.json()?)
        }

        fn json(&self) -> Result<serde_json::Value> {
            let renderer_ref =
                DescriptorIdentity::Renderer(Box::new(self.renderer_descriptor.clone()))
                    .descriptor_ref()?;
            Ok(serde_json::json!({
                "outputs": self.outputs.iter().map(PublicOutputCell::json).collect::<Vec<_>>(),
                "public_schema_id": self.public_schema_id.as_str(),
                "renderer_descriptor_ref": renderer_ref.json(),
            }))
        }
    }

    /// Public output cell declaration.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PublicOutputCell {
        /// Public field path.
        pub public_field_path: PublicFieldPath,
        /// Cell id.
        pub cell_id: CellId,
        /// Producer.
        pub producer: CellProducer,
        /// Scope id.
        pub scope_id: ScopeId,
        /// Semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Schema id.
        pub schema_id: SchemaId,
        /// Value lineage ref.
        pub value_lineage: ValueLineageRef,
        /// Required terminal policy.
        pub required_terminal: RequiredTerminal,
    }

    impl PublicOutputCell {
        fn json(&self) -> serde_json::Value {
            serde_json::json!({
                "cell_id": self.cell_id.as_str(),
                "producer": self.producer.json(),
                "public_field_path": self.public_field_path.as_str(),
                "required_terminal": self.required_terminal.as_str(),
                "schema_id": self.schema_id.as_str(),
                "scope_id": self.scope_id.as_str(),
                "semantic_type_id": self.semantic_type_id.as_str(),
                "value_lineage": self.value_lineage.json(),
            })
        }
    }

    /// Required terminal policy for public outputs and input cells.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum RequiredTerminal {
        /// A concrete value must be produced.
        ProducedOnly,
        /// A typed maybe-skipped terminal is accepted.
        MaybeSkipped,
    }

    impl RequiredTerminal {
        fn as_str(self) -> &'static str {
            match self {
                Self::ProducedOnly => "produced_only",
                Self::MaybeSkipped => "maybe_skipped",
            }
        }
    }

    fn capability_set_json(descriptor: &CapabilitySetDescriptor) -> serde_json::Value {
        serde_json::Value::Array(
            descriptor
                .capabilities
                .iter()
                .map(|capability| {
                    serde_json::json!({
                        "kind": capability.kind.as_str(),
                        "name": capability.name.as_str(),
                        "role": capability.role.as_str(),
                        "version": capability.version.as_str(),
                    })
                })
                .collect(),
        )
    }

    fn remediations_json(
        remediations: &BTreeMap<NodeId, NodeSpec>,
        descriptors: &DescriptorJsonIndex,
    ) -> Result<serde_json::Value> {
        let object = remediations
            .iter()
            .map(|(forward_node_id, node)| {
                Ok((forward_node_id.as_str().to_owned(), node.json(descriptors)?))
            })
            .collect::<Result<serde_json::Map<_, _>>>()?;
        Ok(serde_json::Value::Object(object))
    }

    fn parse_typed_execution_spec(value: &serde_json::Value) -> Result<TypedExecutionSpec> {
        let object = object(value, "typed execution spec")?;
        let spec_version = version::<SpecVersion>(required_str(object, "spec_version")?)?;
        if spec_version.as_str() != SPEC_VERSION {
            return Err(json_error(format!(
                "unsupported spec_version {}",
                spec_version.as_str()
            )));
        }
        let media_type = MediaType::new(required_str(object, "media_type")?)?;
        if media_type.as_str() != MEDIA_TYPE {
            return Err(json_error(format!(
                "unsupported media_type {}",
                media_type.as_str()
            )));
        }
        let canonicalization =
            parse_string::<DigestAlgorithm>(required_str(object, "canonicalization")?)?;
        if canonicalization != DigestAlgorithm::Sha256JcsV1 {
            return Err(json_error(format!(
                "unsupported canonicalization {}",
                canonicalization.as_str()
            )));
        }
        let lowering_version =
            version::<LoweringVersion>(required_str(object, "lowering_version")?)?;
        if lowering_version.as_str() != LOWERING_VERSION {
            return Err(json_error(format!(
                "unsupported lowering_version {}",
                lowering_version.as_str()
            )));
        }

        let descriptor_identities = parse_vec(
            required(object, "descriptor_identities")?,
            parse_descriptor_identity,
        )?;
        let descriptors = DescriptorParseIndex::new(&descriptor_identities)?;
        let nodes = parse_vec_with(required(object, "nodes")?, |node| {
            parse_node_spec(node, &descriptors)
        })?;
        let remediations = parse_remediations(required(object, "remediations")?, &descriptors)?;
        let planning_lineage = parse_vec_with(required(object, "planning_lineage")?, |frame| {
            parse_operation_lineage_frame(frame, &descriptors)
        })?;
        let public_outputs =
            parse_public_output_spec(required(object, "public_outputs")?, &descriptors)?;
        drop(descriptors);

        TypedExecutionSpec::new(TypedExecutionSpecParts {
            authoring: parse_authoring(required(object, "authoring")?)?,
            saga: parse_saga_policy(required(object, "saga")?)?,
            scopes: parse_vec(required(object, "scopes")?, parse_scope_spec)?,
            seeds: parse_vec(required(object, "seeds")?, parse_seed_spec)?,
            descriptor_identities,
            config_refs: parse_vec(required(object, "config_refs")?, parse_config_ref)?,
            nodes,
            remediations,
            cells: parse_vec(required(object, "cells")?, parse_cell_spec)?,
            value_lineages: parse_vec(required(object, "value_lineages")?, parse_value_lineage)?,
            planning_lineage,
            public_outputs,
        })
    }

    fn parse_saga_policy(value: &serde_json::Value) -> Result<SagaPolicySpec> {
        let object = object(value, "saga policy")?;
        match required_str(object, "kind")? {
            "no_side_effects" => Ok(SagaPolicySpec::NoSideEffects),
            "fail_without_acdc_claim" => Ok(SagaPolicySpec::FailWithoutAcdcClaim),
            "manual_resolution" => Ok(SagaPolicySpec::ManualResolution {
                manual: parse_manual_resolution_evidence(required(object, "manual")?)?,
            }),
            "compensate_completed" => Ok(SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: parse_remediation_unresolved(required(
                    object,
                    "on_remediation_unresolved",
                )?)?,
            }),
            kind => Err(json_error(format!("unsupported saga policy kind {kind:?}"))),
        }
    }

    fn parse_remediation_unresolved(
        value: &serde_json::Value,
    ) -> Result<RemediationUnresolvedSpec> {
        let object = object(value, "remediation-unresolved directive")?;
        match required_str(object, "kind")? {
            "manual_resolution" => Ok(RemediationUnresolvedSpec::ManualResolution {
                manual: Box::new(parse_manual_resolution_evidence(required(
                    object, "manual",
                )?)?),
            }),
            "fail_without_acdc_claim" => Ok(RemediationUnresolvedSpec::FailWithoutAcdcClaim),
            kind => Err(json_error(format!(
                "unsupported remediation-unresolved kind {kind:?}"
            ))),
        }
    }

    fn parse_manual_resolution_evidence(
        value: &serde_json::Value,
    ) -> Result<ManualResolutionEvidenceSpec> {
        let object = object(value, "manual-resolution evidence")?;
        Ok(ManualResolutionEvidenceSpec {
            evidence_schema: identity(required_str(object, "evidence_schema")?)?,
            authorization: parse_manual_resolution_authorization(required(
                object,
                "authorization",
            )?)?,
        })
    }

    fn parse_manual_resolution_authorization(
        value: &serde_json::Value,
    ) -> Result<ManualResolutionAuthorizationSpec> {
        let object = object(value, "manual-resolution authorization")?;
        Ok(ManualResolutionAuthorizationSpec {
            verifier_id: ManualAuthorizationVerifierId::new(required_str(object, "verifier_id")?)?,
            signing_scheme: ManualSigningSchemeSpec::new(required_str(object, "signing_scheme")?)?,
            authority: parse_operator_authority_snapshot(required(object, "authority")?)?,
            quorum: parse_manual_authorization_quorum(required(object, "quorum")?)?,
        })
    }

    fn parse_operator_authority_snapshot(
        value: &serde_json::Value,
    ) -> Result<OperatorAuthoritySnapshotSpec> {
        let object = object(value, "operator authority snapshot")?;
        Ok(OperatorAuthoritySnapshotSpec {
            authority_id: OperatorAuthorityId::new(required_str(object, "authority_id")?)?,
            operators: parse_vec(
                required(object, "operators")?,
                parse_operator_authority_member,
            )?,
        })
    }

    fn parse_operator_authority_member(
        value: &serde_json::Value,
    ) -> Result<OperatorAuthorityMemberSpec> {
        let object = object(value, "operator authority member")?;
        Ok(OperatorAuthorityMemberSpec {
            operator_id: OperatorId::new(required_str(object, "operator_id")?)?,
            public_identity: OperatorPublicIdentity::new(required_str(object, "public_identity")?)?,
        })
    }

    fn parse_manual_authorization_quorum(
        value: &serde_json::Value,
    ) -> Result<ManualAuthorizationQuorumSpec> {
        let object = object(value, "manual authorization quorum")?;
        match required_str(object, "kind")? {
            "threshold" => {
                let required_signatures = required_u64(object, "required_signatures")?
                    .try_into()
                    .map_err(|_| {
                    json_error("manual authorization quorum exceeds supported u32 range")
                })?;
                ManualAuthorizationQuorumSpec::new(required_signatures)
            }
            kind => Err(json_error(format!(
                "unsupported manual authorization quorum kind {kind:?}"
            ))),
        }
    }

    fn parse_remediations(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<BTreeMap<NodeId, NodeSpec>> {
        object(value, "remediations")?
            .iter()
            .map(|(forward_node_id, node)| {
                Ok((
                    identity(forward_node_id.as_str())?,
                    parse_node_spec(node, descriptors)?,
                ))
            })
            .collect()
    }

    fn parse_authoring(value: &serde_json::Value) -> Result<AuthoringProvenance> {
        let object = object(value, "authoring")?;
        match required_str(object, "kind")? {
            "operation_expansion" => Ok(AuthoringProvenance::OperationExpansion {
                operation_descriptor_id: identity(required_str(
                    object,
                    "operation_descriptor_id",
                )?)?,
                config_hash: identity(required_str(object, "config_hash")?)?,
            }),
            "state_composition" => Ok(AuthoringProvenance::StateComposition {
                descriptor: parse_composition_descriptor(required(object, "descriptor")?)?,
                config_hash: identity(required_str(object, "config_hash")?)?,
            }),
            "mixed_composition" => Ok(AuthoringProvenance::MixedComposition {
                descriptor: parse_composition_descriptor(required(object, "descriptor")?)?,
                config_hash: identity(required_str(object, "config_hash")?)?,
            }),
            kind => Err(json_error(format!("unsupported authoring kind {kind:?}"))),
        }
    }

    fn parse_composition_descriptor(value: &serde_json::Value) -> Result<CompositionDescriptor> {
        let object = object(value, "composition descriptor")?;
        Ok(CompositionDescriptor {
            descriptor_id: identity(required_str(object, "descriptor_id")?)?,
            name: required_str(object, "name")?.to_owned(),
            version: required_str(object, "version")?.to_owned(),
        })
    }

    fn parse_config_ref(value: &serde_json::Value) -> Result<ConfigRef> {
        let object = object(value, "config ref")?;
        Ok(ConfigRef {
            schema_id: identity(required_str(object, "schema_id")?)?,
            artifact_id: identity(required_str(object, "artifact_id")?)?,
            digest: identity(required_str(object, "digest")?)?,
            byte_len: required_u64(object, "byte_len")?,
            media_type: MediaType::new(required_str(object, "media_type")?)?,
        })
    }

    fn parse_scope_spec(value: &serde_json::Value) -> Result<ScopeSpec> {
        let object = object(value, "scope")?;
        Ok(ScopeSpec {
            scope_id: identity(required_str(object, "scope_id")?)?,
            parent_scope_id: optional_identity(object, "parent_scope_id")?,
            stable_key: StableAuthorKey::new(required_str(object, "stable_key")?)?,
            planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
        })
    }

    fn parse_seed_spec(value: &serde_json::Value) -> Result<SeedSpec> {
        let object = object(value, "seed")?;
        Ok(SeedSpec {
            seed_id: identity(required_str(object, "seed_id")?)?,
            seed_key: StableAuthorKey::new(required_str(object, "seed_key")?)?,
            cell_id: identity(required_str(object, "cell_id")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            semantic_type_id: identity(required_str(object, "semantic_type_id")?)?,
            schema_id: identity(required_str(object, "schema_id")?)?,
            required_digest: optional_identity(object, "required_digest")?,
        })
    }

    fn parse_descriptor_identity(value: &serde_json::Value) -> Result<DescriptorIdentity> {
        let object = object(value, "descriptor identity")?;
        match required_str(object, "descriptor_family")? {
            "state" => Ok(DescriptorIdentity::State(Box::new(
                parse_state_descriptor_identity(value)?,
            ))),
            "operation" => Ok(DescriptorIdentity::Operation(Box::new(
                parse_operation_descriptor_identity(value)?,
            ))),
            "renderer" => Ok(DescriptorIdentity::Renderer(Box::new(
                parse_renderer_descriptor_identity(value)?,
            ))),
            family => Err(json_error(format!(
                "unsupported descriptor_family {family:?}"
            ))),
        }
    }

    struct DescriptorParseIndex<'a> {
        states: BTreeMap<String, &'a StateDescriptorIdentity>,
        operations: BTreeMap<String, &'a OperationDescriptorIdentity>,
        renderers: BTreeMap<String, &'a RendererDescriptorIdentity>,
        refs: BTreeMap<String, DescriptorRef>,
    }

    impl<'a> DescriptorParseIndex<'a> {
        fn new(descriptors: &'a [DescriptorIdentity]) -> Result<Self> {
            let mut index = Self {
                states: BTreeMap::new(),
                operations: BTreeMap::new(),
                renderers: BTreeMap::new(),
                refs: BTreeMap::new(),
            };
            for descriptor in descriptors {
                let reference = descriptor.descriptor_ref()?;
                let key = reference.descriptor_id.as_str().to_owned();
                if index.refs.insert(key.clone(), reference).is_some() {
                    return Err(json_error(format!("duplicate descriptor identity {key}")));
                }
                match descriptor {
                    DescriptorIdentity::State(state) => {
                        index.states.insert(key, state);
                    }
                    DescriptorIdentity::Operation(operation) => {
                        index.operations.insert(key, operation);
                    }
                    DescriptorIdentity::Renderer(renderer) => {
                        index.renderers.insert(key, renderer);
                    }
                }
            }
            Ok(index)
        }

        fn state(&self, reference: &DescriptorRef) -> Result<&'a StateDescriptorIdentity> {
            self.require_ref(reference, DescriptorFamily::State)?;
            self.states
                .get(reference.descriptor_id.as_str())
                .copied()
                .ok_or_else(|| {
                    json_error(format!(
                        "missing state descriptor identity {}",
                        reference.descriptor_id
                    ))
                })
        }

        fn operation(&self, reference: &DescriptorRef) -> Result<&'a OperationDescriptorIdentity> {
            self.require_ref(reference, DescriptorFamily::Operation)?;
            self.operations
                .get(reference.descriptor_id.as_str())
                .copied()
                .ok_or_else(|| {
                    json_error(format!(
                        "missing operation descriptor identity {}",
                        reference.descriptor_id
                    ))
                })
        }

        fn renderer(&self, reference: &DescriptorRef) -> Result<&'a RendererDescriptorIdentity> {
            self.require_ref(reference, DescriptorFamily::Renderer)?;
            self.renderers
                .get(reference.descriptor_id.as_str())
                .copied()
                .ok_or_else(|| {
                    json_error(format!(
                        "missing renderer descriptor identity {}",
                        reference.descriptor_id
                    ))
                })
        }

        fn require_ref(&self, reference: &DescriptorRef, family: DescriptorFamily) -> Result<()> {
            if reference.family != family {
                return Err(json_error(format!(
                    "descriptor {} is {}, expected {}",
                    reference.descriptor_id,
                    reference.family.as_str(),
                    family.as_str()
                )));
            }
            let Some(expected) = self.refs.get(reference.descriptor_id.as_str()) else {
                return Err(json_error(format!(
                    "missing descriptor identity {}",
                    reference.descriptor_id
                )));
            };
            if expected != reference {
                return Err(json_error(format!(
                    "descriptor ref mismatch for {}",
                    reference.descriptor_id
                )));
            }
            Ok(())
        }
    }

    fn parse_descriptor_ref(value: &serde_json::Value) -> Result<DescriptorRef> {
        let object = object(value, "descriptor ref")?;
        Ok(DescriptorRef {
            family: DescriptorFamily::parse(required_str(object, "descriptor_family")?)?,
            descriptor_id: identity(required_str(object, "descriptor_id")?)?,
            descriptor_digest: identity(required_str(object, "descriptor_digest")?)?,
        })
    }

    fn parse_state_descriptor_identity(
        value: &serde_json::Value,
    ) -> Result<StateDescriptorIdentity> {
        let object = object(value, "state descriptor identity")?;
        Ok(StateDescriptorIdentity {
            descriptor_id: identity(required_str(object, "descriptor_id")?)?,
            name: required_str(object, "name")?.to_owned(),
            state_kind: identity(required_str(object, "state_kind")?)?,
            state_version: version(required_str(object, "state_version")?)?,
            config_schema_id: identity(required_str(object, "config_schema_id")?)?,
            input_schema_id: identity(required_str(object, "input_schema_id")?)?,
            output_schema_id: identity(required_str(object, "output_schema_id")?)?,
            output_semantic_type_id: identity(required_str(object, "output_semantic_type_id")?)?,
            effect_kind: identity(required_str(object, "effect_kind")?)?,
            effect_class: required_str(object, "effect_class")?.to_owned(),
            effect_name: required_str(object, "effect_name")?.to_owned(),
            effect_version: version(required_str(object, "effect_version")?)?,
            capabilities: parse_capability_set(required(object, "capabilities")?)?,
            runner: required_str(object, "runner")?.to_owned(),
            side_effect_contract_digest: optional_identity(object, "side_effect_contract_digest")?,
        })
    }

    fn parse_operation_descriptor_identity(
        value: &serde_json::Value,
    ) -> Result<OperationDescriptorIdentity> {
        let object = object(value, "operation descriptor identity")?;
        Ok(OperationDescriptorIdentity {
            descriptor_id: identity(required_str(object, "descriptor_id")?)?,
            name: required_str(object, "name")?.to_owned(),
            operation_kind: identity(required_str(object, "operation_kind")?)?,
            operation_version: version(required_str(object, "operation_version")?)?,
            config_schema_id: identity(required_str(object, "config_schema_id")?)?,
            input_schema_id: identity(required_str(object, "input_schema_id")?)?,
            output_schema_id: identity(required_str(object, "output_schema_id")?)?,
            expansion_abi: required_str(object, "expansion_abi")?.to_owned(),
        })
    }

    fn parse_renderer_descriptor_identity(
        value: &serde_json::Value,
    ) -> Result<RendererDescriptorIdentity> {
        let object = object(value, "renderer descriptor identity")?;
        Ok(RendererDescriptorIdentity {
            descriptor_id: identity(required_str(object, "descriptor_id")?)?,
            renderer_kind: RendererKind::new(required_str(object, "renderer_kind")?)?,
            renderer_version: RendererVersion::new(required_str(object, "renderer_version")?)?,
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
            canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                object,
                "canonicalizer_identity",
            )?)?,
        })
    }

    fn parse_node_spec(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<NodeSpec> {
        let object = object(value, "node")?;
        let descriptor_ref = parse_descriptor_ref(required(object, "descriptor_ref")?)?;
        let descriptor = descriptors.state(&descriptor_ref)?;
        Ok(NodeSpec {
            node_id: identity(required_str(object, "node_id")?)?,
            stable_key: StableAuthorKey::new(required_str(object, "stable_key")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            state_kind: descriptor.state_kind.clone(),
            state_version: descriptor.state_version.clone(),
            descriptor_id: descriptor.descriptor_id.clone(),
            config_ref: parse_config_ref(required(object, "config_ref")?)?,
            input_bindings: parse_input_binding_spec(required(object, "input_bindings")?)?,
            output_cell: identity(required_str(object, "output_cell")?)?,
            effect_kind: descriptor.effect_kind.clone(),
            capability_bindings: descriptor.capabilities.clone(),
            adapter_bindings: parse_vec(
                required(object, "adapter_bindings")?,
                parse_adapter_binding,
            )?,
            side_effect: optional_parse(object, "side_effect", parse_side_effect_contract)?,
            framework: optional_parse_with(object, "framework", |framework| {
                parse_framework_node(framework, descriptors)
            })?,
            planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
            deterministic_predecessors: parse_identity_vec(required(
                object,
                "deterministic_predecessors",
            )?)?,
        })
    }

    fn parse_adapter_binding(value: &serde_json::Value) -> Result<AdapterBinding> {
        let object = object(value, "adapter binding")?;
        Ok(AdapterBinding {
            adapter_kind: identity(required_str(object, "adapter_kind")?)?,
            adapter_version: version(required_str(object, "adapter_version")?)?,
            binding_digest: optional_identity(object, "binding_digest")?,
        })
    }

    fn parse_side_effect_contract(value: &serde_json::Value) -> Result<SideEffectContractSpec> {
        let object = object(value, "side-effect contract")?;
        Ok(SideEffectContractSpec {
            contract_digest: identity(required_str(object, "contract_digest")?)?,
            resource_claim: parse_resource_claim(required(object, "resource_claim")?)?,
            verification: parse_side_effect_verification(required(object, "verification")?)?,
        })
    }

    fn parse_side_effect_verification(
        value: &serde_json::Value,
    ) -> Result<SideEffectVerificationSpec> {
        let verification = object(value, "side-effect verification")?;
        match required_str(verification, "kind")? {
            "receipt" => Ok(SideEffectVerificationSpec::Receipt),
            "finalized" => {
                let finalized = object(
                    required(verification, "finalized")?,
                    "finalized side-effect verification",
                )?;
                let depth = required_u64(finalized, "depth")?;
                if depth == 0 {
                    return Err(json_error(
                        "finalized side-effect verification depth must be positive",
                    ));
                }
                Ok(SideEffectVerificationSpec::Finalized { depth })
            }
            kind => Err(json_error(format!(
                "unsupported side-effect verification kind {kind:?}"
            ))),
        }
    }

    fn parse_resource_claim(value: &serde_json::Value) -> Result<ResourceClaimSpec> {
        let claim = object(value, "resource claim")?;
        match required_str(claim, "kind")? {
            "exclusive" => {
                let exclusive = object(required(claim, "exclusive")?, "exclusive resource claim")?;
                Ok(ResourceClaimSpec::Exclusive {
                    namespace: ResourceNamespace::new(required_str(exclusive, "namespace")?)?,
                    key_schema: identity(required_str(exclusive, "key_schema")?)?,
                })
            }
            "exact_touched_set" => {
                let touched_set = object(
                    required(claim, "exact_touched_set")?,
                    "exact-touched-set resource claim",
                )?;
                Ok(ResourceClaimSpec::ExactTouchedSet {
                    namespace: ResourceNamespace::new(required_str(touched_set, "namespace")?)?,
                    evidence_schema: identity(required_str(touched_set, "evidence_schema")?)?,
                })
            }
            "manual_only" => Ok(ResourceClaimSpec::ManualOnly),
            kind => Err(json_error(format!(
                "unsupported resource claim kind {kind:?}"
            ))),
        }
    }

    fn parse_framework_node(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<FrameworkNodeSpec> {
        let object = object(value, "framework node")?;
        match required_str(object, "kind")? {
            "bridge" => Ok(FrameworkNodeSpec::Bridge(parse_bridge_node(required(
                object, "bridge",
            )?)?)),
            "side_effect_verify" => Ok(FrameworkNodeSpec::SideEffectVerify(
                parse_side_effect_verify_node(required(object, "side_effect_verify")?)?,
            )),
            "public_output_render" => Ok(FrameworkNodeSpec::PublicOutputRender(
                parse_public_output_render_node(
                    required(object, "public_output_render")?,
                    descriptors,
                )?,
            )),
            "project_retention_manifest" => Ok(FrameworkNodeSpec::ProjectRetentionManifest(
                parse_project_retention_manifest_node(required(
                    object,
                    "project_retention_manifest",
                )?)?,
            )),
            "complete_run" => Ok(FrameworkNodeSpec::CompleteRun(parse_complete_run_node(
                required(object, "complete_run")?,
            )?)),
            "resolve_saga_terminal" => Ok(FrameworkNodeSpec::ResolveSagaTerminal(
                parse_resolve_saga_terminal_node(required(object, "resolve_saga_terminal")?)?,
            )),
            kind => Err(json_error(format!("unsupported framework kind {kind:?}"))),
        }
    }

    fn parse_side_effect_verify_node(
        value: &serde_json::Value,
    ) -> Result<SideEffectVerifyNodeSpec> {
        let object = object(value, "side-effect verify node")?;
        Ok(SideEffectVerifyNodeSpec {
            pair_id: identity(required_str(object, "pair_id")?)?,
            submit_node_id: identity(required_str(object, "submit_node_id")?)?,
        })
    }

    fn parse_bridge_node(value: &serde_json::Value) -> Result<BridgeNodeSpec> {
        let object = object(value, "bridge node")?;
        Ok(BridgeNodeSpec {
            bridge_kind: parse_bridge_kind(required_str(object, "bridge_kind")?)?,
            source_scope_id: identity(required_str(object, "source_scope_id")?)?,
            target_scope_id: identity(required_str(object, "target_scope_id")?)?,
            source_cell_id: identity(required_str(object, "source_cell_id")?)?,
            target_cell_id: identity(required_str(object, "target_cell_id")?)?,
            semantic_type_id: identity(required_str(object, "semantic_type_id")?)?,
            schema_id: identity(required_str(object, "schema_id")?)?,
            policy: parse_bridge_policy(required_str(object, "policy")?)?,
            provenance: parse_bridge_provenance(required_str(object, "provenance")?)?,
        })
    }

    fn parse_public_output_render_node(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<PublicOutputRenderNodeSpec> {
        let object = object(value, "public-output render node")?;
        let renderer_ref = parse_descriptor_ref(required(object, "renderer_descriptor_ref")?)?;
        let renderer_descriptor = descriptors.renderer(&renderer_ref)?.clone();
        Ok(PublicOutputRenderNodeSpec {
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
            output_spec_digest: identity(required_str(object, "output_spec_digest")?)?,
            renderer_descriptor,
            required_cells: parse_vec(required(object, "required_cells")?, parse_public_cell)?,
        })
    }

    fn parse_project_retention_manifest_node(
        value: &serde_json::Value,
    ) -> Result<ProjectRetentionManifestNodeSpec> {
        let object = object(value, "project-retention-manifest node")?;
        Ok(ProjectRetentionManifestNodeSpec {
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
            public_output_receipt_cell: identity(required_str(
                object,
                "public_output_receipt_cell",
            )?)?,
        })
    }

    fn parse_complete_run_node(value: &serde_json::Value) -> Result<CompleteRunNodeSpec> {
        let object = object(value, "complete-run node")?;
        Ok(CompleteRunNodeSpec {
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
            retention_manifest_receipt_cell: identity(required_str(
                object,
                "retention_manifest_receipt_cell",
            )?)?,
        })
    }

    fn parse_resolve_saga_terminal_node(
        value: &serde_json::Value,
    ) -> Result<ResolveSagaTerminalNodeSpec> {
        let object = object(value, "resolve-saga-terminal node")?;
        Ok(ResolveSagaTerminalNodeSpec {
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
        })
    }

    fn parse_input_binding_spec(value: &serde_json::Value) -> Result<InputBindingSpec> {
        let object = object(value, "input binding")?;
        Ok(InputBindingSpec {
            input_schema_id: identity(required_str(object, "input_schema_id")?)?,
            input_descriptor_id: identity(required_str(object, "input_descriptor_id")?)?,
            root: parse_input_binding_node(required(object, "root")?)?,
            digest: identity(required_str(object, "digest")?)?,
        })
    }

    fn parse_input_binding_node(value: &serde_json::Value) -> Result<InputBindingNodeSpec> {
        let object = object(value, "input binding node")?;
        match required_str(object, "kind")? {
            "unit" => Ok(InputBindingNodeSpec::Unit),
            "cell" => Ok(InputBindingNodeSpec::Cell(Box::new(
                parse_input_binding_cell(value)?,
            ))),
            "tuple" => Ok(InputBindingNodeSpec::Tuple(parse_vec(
                required(object, "elements")?,
                parse_input_binding_node,
            )?)),
            "struct" => Ok(InputBindingNodeSpec::Struct(parse_vec(
                required(object, "fields")?,
                parse_named_input_binding,
            )?)),
            "vec" => Ok(InputBindingNodeSpec::Vec {
                elements: parse_vec(required(object, "elements")?, parse_input_binding_node)?,
                ordering: parse_ordering(required_str(object, "ordering")?)?,
                domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
            }),
            "non_empty_vec" => Ok(InputBindingNodeSpec::NonEmptyVec {
                elements: parse_vec(required(object, "elements")?, parse_input_binding_node)?,
                ordering: parse_ordering(required_str(object, "ordering")?)?,
                domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
            }),
            kind => Err(json_error(format!(
                "unsupported input binding kind {kind:?}"
            ))),
        }
    }

    fn parse_input_binding_cell(value: &serde_json::Value) -> Result<InputBindingCellSpec> {
        let object = object(value, "input binding cell")?;
        Ok(InputBindingCellSpec {
            field_path: PublicFieldPath::new(required_str(object, "field_path")?)?,
            cell_id: identity(required_str(object, "cell_id")?)?,
            semantic_type_id: identity(required_str(object, "semantic_type_id")?)?,
            schema_id: identity(required_str(object, "schema_id")?)?,
            required_terminal: parse_required_terminal(required_str(object, "required_terminal")?)?,
            value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
        })
    }

    fn parse_named_input_binding(value: &serde_json::Value) -> Result<NamedInputBindingSpec> {
        let object = object(value, "named input binding")?;
        Ok(NamedInputBindingSpec {
            field_path: PublicFieldPath::new(required_str(object, "field_path")?)?,
            node: parse_input_binding_node(required(object, "node")?)?,
        })
    }

    fn parse_domain_key_ref(value: &serde_json::Value) -> Result<StableDomainKeyRef> {
        let object = object(value, "stable domain key")?;
        Ok(StableDomainKeyRef {
            schema_id: identity(required_str(object, "schema_id")?)?,
            content_digest: identity(required_str(object, "content_digest")?)?,
        })
    }

    fn parse_cell_producer(value: &serde_json::Value) -> Result<CellProducer> {
        let object = object(value, "cell producer")?;
        match required_str(object, "kind")? {
            "node" => Ok(CellProducer::Node(identity(required_str(
                object, "node_id",
            )?)?)),
            "seed" => Ok(CellProducer::Seed(identity(required_str(
                object, "seed_id",
            )?)?)),
            kind => Err(json_error(format!(
                "unsupported cell producer kind {kind:?}"
            ))),
        }
    }

    fn parse_cell_spec(value: &serde_json::Value) -> Result<CellSpec> {
        let object = object(value, "cell")?;
        Ok(CellSpec {
            cell_id: identity(required_str(object, "cell_id")?)?,
            producer: parse_cell_producer(required(object, "producer")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            semantic_type_id: identity(required_str(object, "semantic_type_id")?)?,
            schema_id: identity(required_str(object, "schema_id")?)?,
            value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
            terminal_policy: parse_cell_terminal_policy(required_str(object, "terminal_policy")?)?,
            storage_policy: parse_storage_policy(required_str(object, "storage_policy")?)?,
            redaction_policy: parse_redaction_policy(required_str(object, "redaction_policy")?)?,
        })
    }

    fn parse_value_lineage_ref(value: &serde_json::Value) -> Result<ValueLineageRef> {
        let object = object(value, "value lineage ref")?;
        Ok(ValueLineageRef {
            lineage_digest: identity(required_str(object, "lineage_digest")?)?,
        })
    }

    fn parse_value_lineage(value: &serde_json::Value) -> Result<ValueLineage> {
        let object = object(value, "value lineage")?;
        Ok(ValueLineage {
            lineage_ref: parse_value_lineage_ref(required(object, "lineage_ref")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            producer: parse_cell_producer(required(object, "producer")?)?,
            input_cells: parse_identity_vec(required(object, "input_cells")?)?,
            config_ref_digest: optional_identity(object, "config_ref_digest")?,
            planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
            domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
            transform_policy: parse_transform_policy(required_str(object, "transform_policy")?)?,
        })
    }

    fn parse_planning_lineage(value: &serde_json::Value) -> Result<PlanningLineage> {
        let object = object(value, "planning lineage")?;
        Ok(PlanningLineage {
            active_operation_instances: parse_identity_vec(required(
                object,
                "active_operation_instances",
            )?)?,
            completed_operation_frames: parse_identity_vec(required(
                object,
                "completed_operation_frames",
            )?)?,
            lineage_digest: identity(required_str(object, "lineage_digest")?)?,
        })
    }

    fn parse_operation_lineage_frame(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<OperationLineageFrameSpec> {
        let object = object(value, "operation lineage frame")?;
        let descriptor_ref = parse_descriptor_ref(required(object, "operation_descriptor_ref")?)?;
        let descriptor = descriptors.operation(&descriptor_ref)?;
        Ok(OperationLineageFrameSpec {
            operation_instance_id: identity(required_str(object, "operation_instance_id")?)?,
            operation_key: StableAuthorKey::new(required_str(object, "operation_key")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            operation_descriptor_id: descriptor.descriptor_id.clone(),
            config_ref_digest: identity(required_str(object, "config_ref_digest")?)?,
            input_bindings: parse_input_binding_spec(required(object, "input_bindings")?)?,
            input_binding_digest: identity(required_str(object, "input_binding_digest")?)?,
            parent_planning_lineage: parse_planning_lineage(required(
                object,
                "parent_planning_lineage",
            )?)?,
            output_cells: parse_identity_vec(required(object, "output_cells")?)?,
            lineage_digest: identity(required_str(object, "lineage_digest")?)?,
        })
    }

    fn parse_public_output_spec(
        value: &serde_json::Value,
        descriptors: &DescriptorParseIndex<'_>,
    ) -> Result<PublicOutputSpec> {
        let object = object(value, "public output spec")?;
        let renderer_ref = parse_descriptor_ref(required(object, "renderer_descriptor_ref")?)?;
        let renderer_descriptor = descriptors.renderer(&renderer_ref)?.clone();
        Ok(PublicOutputSpec {
            public_schema_id: identity(required_str(object, "public_schema_id")?)?,
            outputs: parse_vec(required(object, "outputs")?, parse_public_cell)?,
            renderer_descriptor,
        })
    }

    fn parse_public_cell(value: &serde_json::Value) -> Result<PublicOutputCell> {
        let object = object(value, "public output cell")?;
        Ok(PublicOutputCell {
            public_field_path: PublicFieldPath::new(required_str(object, "public_field_path")?)?,
            cell_id: identity(required_str(object, "cell_id")?)?,
            producer: parse_cell_producer(required(object, "producer")?)?,
            scope_id: identity(required_str(object, "scope_id")?)?,
            semantic_type_id: identity(required_str(object, "semantic_type_id")?)?,
            schema_id: identity(required_str(object, "schema_id")?)?,
            value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
            required_terminal: parse_required_terminal(required_str(object, "required_terminal")?)?,
        })
    }

    fn parse_capability_set(value: &serde_json::Value) -> Result<CapabilitySetDescriptor> {
        let capabilities = array(value, "capabilities")?
            .iter()
            .map(parse_capability_descriptor)
            .collect::<Result<Vec<_>>>()?;
        Ok(CapabilitySetDescriptor::new(capabilities)?)
    }

    fn parse_capability_descriptor(value: &serde_json::Value) -> Result<CapabilityDescriptor> {
        let object = object(value, "capability descriptor")?;
        Ok(CapabilityDescriptor::new(
            identity(required_str(object, "kind")?)?,
            version(required_str(object, "version")?)?,
            parse_capability_role(required_str(object, "role")?)?,
            required_str(object, "name")?,
        )?)
    }

    fn parse_capability_role(value: &str) -> Result<CapabilityRole> {
        match value {
            "read_external" => Ok(CapabilityRole::ReadExternal),
            "managed_platform_write" => Ok(CapabilityRole::ManagedPlatformWrite),
            "support" => Ok(CapabilityRole::Support),
            "external_mutation_authority" => Ok(CapabilityRole::ExternalMutationAuthority),
            role => Err(json_error(format!("unsupported capability role {role:?}"))),
        }
    }

    fn parse_bridge_kind(value: &str) -> Result<BridgeKind> {
        match value {
            "import_from_parent" => Ok(BridgeKind::ImportFromParent),
            "export_to_parent" => Ok(BridgeKind::ExportToParent),
            kind => Err(json_error(format!("unsupported bridge kind {kind:?}"))),
        }
    }

    fn parse_bridge_policy(value: &str) -> Result<BridgePolicy> {
        match value {
            "same_run_same_value" => Ok(BridgePolicy::SameRunSameValue),
            policy => Err(json_error(format!("unsupported bridge policy {policy:?}"))),
        }
    }

    fn parse_bridge_provenance(value: &str) -> Result<BridgeProvenance> {
        match value {
            "framework_child_scope_v1" => Ok(BridgeProvenance::FrameworkChildScopeV1),
            provenance => Err(json_error(format!(
                "unsupported bridge provenance {provenance:?}"
            ))),
        }
    }

    fn parse_ordering(value: &str) -> Result<OrderingEvidence> {
        match value {
            "explicit_author_order" => Ok(OrderingEvidence::ExplicitAuthorOrder),
            "stable_domain_key" => Ok(OrderingEvidence::StableDomainKey),
            ordering => Err(json_error(format!("unsupported ordering {ordering:?}"))),
        }
    }

    fn parse_cell_terminal_policy(value: &str) -> Result<CellTerminalPolicy> {
        match value {
            "produced_only" => Ok(CellTerminalPolicy::ProducedOnly),
            "maybe_skipped" => Ok(CellTerminalPolicy::MaybeSkipped),
            policy => Err(json_error(format!(
                "unsupported cell terminal policy {policy:?}"
            ))),
        }
    }

    fn parse_storage_policy(value: &str) -> Result<StoragePolicy> {
        match value {
            "content_addressed" => Ok(StoragePolicy::ContentAddressed),
            "artifact_reference" => Ok(StoragePolicy::ArtifactReference),
            "public_output_artifact" => Ok(StoragePolicy::PublicOutputArtifact),
            policy => Err(json_error(format!("unsupported storage policy {policy:?}"))),
        }
    }

    fn parse_redaction_policy(value: &str) -> Result<RedactionPolicy> {
        match value {
            "public" => Ok(RedactionPolicy::Public),
            "redacted" => Ok(RedactionPolicy::Redacted),
            policy => Err(json_error(format!(
                "unsupported redaction policy {policy:?}"
            ))),
        }
    }

    fn parse_transform_policy(value: &str) -> Result<LineageTransformPolicy> {
        match value {
            "source" => Ok(LineageTransformPolicy::Source),
            "state_output" => Ok(LineageTransformPolicy::StateOutput),
            "same_value_bridge" => Ok(LineageTransformPolicy::SameValueBridge),
            policy => Err(json_error(format!(
                "unsupported lineage transform policy {policy:?}"
            ))),
        }
    }

    fn parse_required_terminal(value: &str) -> Result<RequiredTerminal> {
        match value {
            "produced_only" => Ok(RequiredTerminal::ProducedOnly),
            "maybe_skipped" => Ok(RequiredTerminal::MaybeSkipped),
            terminal => Err(json_error(format!(
                "unsupported required terminal {terminal:?}"
            ))),
        }
    }

    fn parse_vec<T>(
        value: &serde_json::Value,
        parser: fn(&serde_json::Value) -> Result<T>,
    ) -> Result<Vec<T>> {
        array(value, "array")?.iter().map(parser).collect()
    }

    fn parse_vec_with<T>(
        value: &serde_json::Value,
        parser: impl Fn(&serde_json::Value) -> Result<T>,
    ) -> Result<Vec<T>> {
        array(value, "array")?.iter().map(parser).collect()
    }

    fn parse_identity_vec<T>(value: &serde_json::Value) -> Result<Vec<T>>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        array(value, "identity array")?
            .iter()
            .map(|value| identity(string(value, "identity")?))
            .collect()
    }

    fn optional_parse<T>(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &'static str,
        parser: fn(&serde_json::Value) -> Result<T>,
    ) -> Result<Option<T>> {
        match object.get(field) {
            Some(serde_json::Value::Null) | None => Ok(None),
            Some(value) => parser(value).map(Some),
        }
    }

    fn optional_parse_with<T>(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &'static str,
        parser: impl Fn(&serde_json::Value) -> Result<T>,
    ) -> Result<Option<T>> {
        match object.get(field) {
            Some(serde_json::Value::Null) | None => Ok(None),
            Some(value) => parser(value).map(Some),
        }
    }

    fn optional_identity<T>(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &'static str,
    ) -> Result<Option<T>>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        match object.get(field) {
            Some(serde_json::Value::Null) | None => Ok(None),
            Some(value) => identity(string(value, field)?).map(Some),
        }
    }

    fn identity<T>(value: &str) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        parse_string(value)
    }

    fn version<T>(value: &str) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        parse_string(value)
    }

    fn parse_string<T>(value: &str) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        match value.parse::<T>() {
            Ok(parsed) => Ok(parsed),
            Err(error) => Err(SpecError::Identity(error.to_string())),
        }
    }

    fn object<'a>(
        value: &'a serde_json::Value,
        context: &'static str,
    ) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
        value
            .as_object()
            .ok_or_else(|| json_error(format!("{context} must be a JSON object")))
    }

    fn array<'a>(
        value: &'a serde_json::Value,
        context: &'static str,
    ) -> Result<&'a Vec<serde_json::Value>> {
        value
            .as_array()
            .ok_or_else(|| json_error(format!("{context} must be a JSON array")))
    }

    fn required<'a>(
        object: &'a serde_json::Map<String, serde_json::Value>,
        field: &'static str,
    ) -> Result<&'a serde_json::Value> {
        object
            .get(field)
            .ok_or_else(|| json_error(format!("missing required field {field}")))
    }

    fn required_str<'a>(
        object: &'a serde_json::Map<String, serde_json::Value>,
        field: &'static str,
    ) -> Result<&'a str> {
        string(required(object, field)?, field)
    }

    fn required_u64(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &'static str,
    ) -> Result<u64> {
        required(object, field)?
            .as_u64()
            .ok_or_else(|| json_error(format!("{field} must be an unsigned integer")))
    }

    fn string<'a>(value: &'a serde_json::Value, field: &'static str) -> Result<&'a str> {
        value
            .as_str()
            .ok_or_else(|| json_error(format!("{field} must be a string")))
    }

    fn json_error(message: impl Into<String>) -> SpecError {
        SpecError::Json(message.into())
    }

    #[cfg(test)]
    mod tests;
}
