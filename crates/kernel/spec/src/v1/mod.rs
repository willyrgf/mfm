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
#[path = "nodes.rs"]
mod nodes;
pub use self::nodes::*;
#[path = "authoring.rs"]
mod authoring;
pub use self::authoring::*;

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

#[cfg(test)]
mod tests;
