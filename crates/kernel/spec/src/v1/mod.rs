use std::collections::BTreeMap;

use super::{
    canonical_json, content_digest, spec_hash_from_canonical, CheckedFieldPath,
    CheckedResourceNamespace, CheckedStableAuthorKey, CheckedVisibleAscii256, ContentDigest,
    ContextDescriptorId, ContextRef, ContextResourceKind, ContextStage, DigestAlgorithm,
    PlainCanonicalJsonBytes, Result, SideEffectVerifyPairErrorKind, SpecError, SpecHash,
};
use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CellId, DescriptorId, EffectKind, EffectVersion,
    LoweringVersion, NodeId, OperationInstanceId, OperationKind, OperationVersion, SchemaId,
    ScopeId, SeedId, SemanticTypeId, SideEffectPairId, SpecVersion, StateKind, StateVersion,
};

#[path = "parser.rs"]
mod parser;
use self::parser::{
    capability_set_json, json_error, parse_typed_execution_spec, remediations_json,
    require_context_ref, validate_input_context_refs,
};
#[path = "data_model.rs"]
mod data_model;
pub use self::data_model::{
    CellProducer, CellSpec, CellTerminalPolicy, InputBindingCellSpec, InputBindingNodeSpec,
    InputBindingSpec, LineageTransformPolicy, NamedInputBindingSpec, OperationLineageFrameSpec,
    OrderingEvidence, PlanningLineage, PublicOutputCell, PublicOutputSpec, RedactionPolicy,
    RequiredTerminal, StableDomainKeyRef, StoragePolicy, ValueLineage, ValueLineageRef,
};
#[path = "context.rs"]
mod context;
pub use self::context::*;
#[path = "framework_bindings.rs"]
mod framework_bindings;
pub use self::framework_bindings::*;

#[path = "descriptors.rs"]
mod descriptors;
pub(crate) use self::descriptors::DescriptorJsonIndex;
pub use self::descriptors::{
    DescriptorFamily, DescriptorIdentity, DescriptorRef, FactDescriptorRef,
    OperationDescriptorIdentity, RendererDescriptorIdentity, StateDescriptorIdentity,
};

/// v1 spec-version string.
pub const SPEC_VERSION: &str = "mfm.typed.execution_spec.v1";
/// v1 typed execution spec media type.
pub const MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";
/// v1 lowering-version string.
pub const LOWERING_VERSION: &str = "mfm.typed.lowering.v2";

pub use super::{
    complete_run_receipt_schema_id, complete_run_receipt_semantic_type_id,
    public_output_receipt_schema_id, public_output_receipt_semantic_type_id,
    resolve_saga_terminal_receipt_schema_id, resolve_saga_terminal_receipt_semantic_type_id,
    retention_manifest_receipt_schema_id, retention_manifest_receipt_semantic_type_id,
    typed_execution_spec_schema_id,
};

checked_string_type!(
    /// Checked media type string.
    MediaType,
    "media type",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Stable author key persisted in specs.
    StableAuthorKey,
    "stable author key",
    CheckedStableAuthorKey
);
checked_string_type!(
    /// Public output field path persisted in specs.
    PublicFieldPath,
    "public field path",
    CheckedFieldPath
);
checked_string_type!(
    /// Stable renderer kind string.
    RendererKind,
    "renderer kind",
    CheckedStableAuthorKey
);
checked_string_type!(
    /// Stable renderer version string.
    RendererVersion,
    "renderer version",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Canonicalizer identity string used by a renderer.
    CanonicalizerIdentity,
    "canonicalizer identity",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Capability-style resource namespace for cross-run resource claims.
    ResourceNamespace,
    "resource namespace",
    CheckedResourceNamespace
);
checked_string_type!(
    /// Manual authorization verifier identity certified for manual saga resolution.
    ManualAuthorizationVerifierId,
    "manual authorization verifier id",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Operator authority snapshot identity certified for manual saga resolution.
    OperatorAuthorityId,
    "operator authority id",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Stable operator identity inside a certified operator authority snapshot.
    OperatorId,
    "operator id",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Public operator identity that may authorize a manual saga resolution.
    OperatorPublicIdentity,
    "operator public identity",
    CheckedVisibleAscii256
);
checked_string_type!(
    /// Manual-resolution signing scheme identifier.
    ManualSigningSchemeSpec,
    "manual signing scheme",
    CheckedVisibleAscii256
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
    /// Certified transition contexts available to nodes and cells.
    pub contexts: Vec<CertifiedContextSpec>,
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
        let spec = Self {
            spec_version: SpecVersion::new(SPEC_VERSION)?,
            media_type: MediaType::new(MEDIA_TYPE)?,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            lowering_version: LoweringVersion::new(LOWERING_VERSION)?,
            authoring: parts.authoring,
            saga: parts.saga,
            contexts: parts.contexts,
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
        };
        spec.validate_context_references()?;
        Ok(spec)
    }

    /// Returns canonical JSON bytes for the hash-defining spec.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(self.json()?)
    }

    /// Returns the spec hash over canonical `TypedExecutionSpec` bytes.
    pub fn spec_hash(&self) -> Result<SpecHash> {
        Ok(spec_hash_from_canonical(&self.canonical_json()?))
    }

    /// Decodes a persisted v1 typed execution spec from canonical or non-canonical JSON.
    ///
    /// The returned value is reconstructed through checked typed constructors and can be
    /// re-hashed through [`Self::spec_hash`]. Callers that load a run-start artifact must
    /// compare the recomputed hash with the `RunAdmitted.spec_hash` stored in the typed stream.
    pub fn from_json_str(input: &str) -> Result<Self> {
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
        self.validate_context_references()?;
        let descriptor_index = DescriptorJsonIndex::new(&self.descriptor_identities)?;
        Ok(serde_json::json!({
            "authoring": self.authoring.json(),
            "canonicalization": self.canonicalization.as_str(),
            "cells": self.cells.iter().map(CellSpec::json).collect::<Vec<_>>(),
            "config_refs": self.config_refs.iter().map(ConfigRef::json).collect::<Vec<_>>(),
            "contexts": self.contexts.iter().map(CertifiedContextSpec::json).collect::<Result<Vec<_>>>()?,
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

    fn validate_context_references(&self) -> Result<()> {
        let mut refs = BTreeMap::new();
        for context in &self.contexts {
            if refs
                .insert(context.context_ref.as_str().to_owned(), ())
                .is_some()
            {
                return Err(json_error(format!(
                    "duplicate certified context {}",
                    context.context_ref
                )));
            }
            context.validate_digest_and_ref()?;
        }

        for node in &self.nodes {
            node.context.validate_known_ref(&refs)?;
            validate_input_context_refs(&node.input_bindings.root, &refs)?;
        }
        for node in self.remediations.values() {
            node.context.validate_known_ref(&refs)?;
            validate_input_context_refs(&node.input_bindings.root, &refs)?;
        }
        for cell in &self.cells {
            cell.context.validate_known_ref(&refs)?;
        }

        Ok(())
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
    /// Certified transition contexts available to nodes and cells.
    pub contexts: Vec<CertifiedContextSpec>,
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
    /// Certified transition-context requirement.
    pub context: NodeContextSpec,
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
    /// Fact descriptors this producing node may emit.
    pub fact_descriptor_allowlist: Vec<FactDescriptorRef>,
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
        let descriptor_ref = descriptors.require(&self.descriptor_id, DescriptorFamily::State)?;
        Ok(serde_json::json!({
            "adapter_bindings": self.adapter_bindings.iter().map(AdapterBinding::json).collect::<Vec<_>>(),
            "config_ref": self.config_ref.json(),
            "context": self.context.json(),
            "descriptor_ref": descriptor_ref.json(),
            "deterministic_predecessors": self.deterministic_predecessors
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            "fact_descriptor_allowlist": self.fact_descriptor_allowlist
                .iter()
                .map(FactDescriptorRef::json)
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
        let renderer_ref = DescriptorIdentity::Renderer(Box::new(self.renderer_descriptor.clone()))
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

#[cfg(test)]
mod tests;
