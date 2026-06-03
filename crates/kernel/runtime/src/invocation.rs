use std::collections::BTreeMap;

use mfm_capabilities::{CapabilityDescriptor, CapabilitySetDescriptor};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, NodeId, RunId, SchemaId, SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::CertifiedRuntimeSpec;

/// Prepared, store-verified invocation supplied to an erased node runner.
///
/// The runtime constructs this value only after revalidating the latest run stream against the
/// certified spec. It carries committed config evidence, materialized input evidence, certified
/// capability descriptors, and recovery facts into the runner without exposing a public
/// constructor.
pub struct PreparedRunnerInvocation<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) spec_hash: &'a SpecHash,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) descriptor: &'a spec::StateDescriptorIdentity,
    pub(crate) output_cell: &'a spec::CellSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) attempt_no: u32,
    pub(crate) config_artifact: store::ArtifactEvidenceRef,
    pub(crate) inputs: MaterializedInputs,
    pub(crate) caps: CertifiedRuntimeCapabilities,
    pub(crate) recorded_facts: RecordedFacts,
    pub(crate) projections: &'a store::ProjectionSnapshot,
    pub(crate) run_stream: &'a [store::KernelEventEnvelope],
}

impl<'a> PreparedRunnerInvocation<'a> {
    /// Run id being executed.
    pub fn run_id(&self) -> &'a RunId {
        self.run_id
    }

    /// Certified typed spec hash.
    pub fn spec_hash(&self) -> &'a SpecHash {
        self.spec_hash
    }

    /// Certified node spec.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.node
    }

    /// Certified state descriptor identity for the node.
    pub fn descriptor(&self) -> &'a spec::StateDescriptorIdentity {
        self.descriptor
    }

    /// Certified output cell spec for the node.
    pub fn output_cell(&self) -> &'a spec::CellSpec {
        self.output_cell
    }

    /// Store-owned attempt id minted by the scheduler.
    pub fn attempt_id(&self) -> &'a AttemptId {
        self.attempt_id
    }

    /// Attempt number for this node.
    pub const fn attempt_no(&self) -> u32 {
        self.attempt_no
    }

    /// Store-committed typed config artifact evidence matching the certified config ref.
    pub fn config_artifact(&self) -> &store::ArtifactEvidenceRef {
        &self.config_artifact
    }

    /// Materialized input evidence derived only from certified cells and validated stream state.
    pub fn inputs(&self) -> &MaterializedInputs {
        &self.inputs
    }

    /// Runtime capabilities minted only from the certified node capability set.
    pub fn caps(&self) -> &CertifiedRuntimeCapabilities {
        &self.caps
    }

    /// Facts already committed for this attempt and therefore reusable after recovery.
    pub fn recorded_facts(&self) -> &RecordedFacts {
        &self.recorded_facts
    }

    /// Store-owned projection snapshot observed before the runner invocation.
    pub fn projections(&self) -> &store::ProjectionSnapshot {
        self.projections
    }

    pub(crate) fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.runtime_spec
    }

    pub(crate) fn run_stream(&self) -> &'a [store::KernelEventEnvelope] {
        self.run_stream
    }
}

/// Context supplied to an erased node runner.
pub struct ErasedRunCtx<'a> {
    invocation: &'a PreparedRunnerInvocation<'a>,
}

impl<'a> ErasedRunCtx<'a> {
    pub(crate) fn from_prepared(invocation: &'a PreparedRunnerInvocation<'a>) -> Self {
        Self { invocation }
    }

    /// Prepared runner invocation backing this context.
    pub fn invocation(&self) -> &'a PreparedRunnerInvocation<'a> {
        self.invocation
    }

    /// Run id being executed.
    pub fn run_id(&self) -> &'a RunId {
        self.invocation.run_id()
    }

    /// Certified typed spec hash.
    pub fn spec_hash(&self) -> &'a SpecHash {
        self.invocation.spec_hash()
    }

    /// Certified node spec.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.invocation.node()
    }

    /// Certified state descriptor identity for the node.
    pub fn descriptor(&self) -> &'a spec::StateDescriptorIdentity {
        self.invocation.descriptor()
    }

    /// Certified output cell spec for the node.
    pub fn output_cell(&self) -> &'a spec::CellSpec {
        self.invocation.output_cell()
    }

    /// Store-owned attempt id minted by the scheduler.
    pub fn attempt_id(&self) -> &'a AttemptId {
        self.invocation.attempt_id()
    }

    /// Attempt number for this node.
    pub const fn attempt_no(&self) -> u32 {
        self.invocation.attempt_no()
    }

    /// Store-committed typed config artifact evidence matching the certified config ref.
    pub fn config_artifact(&self) -> &store::ArtifactEvidenceRef {
        self.invocation.config_artifact()
    }

    /// Materialized input evidence derived only from certified cells and validated stream state.
    pub fn inputs(&self) -> &MaterializedInputs {
        self.invocation.inputs()
    }

    /// Runtime capabilities minted only from the certified node capability set.
    pub fn caps(&self) -> &CertifiedRuntimeCapabilities {
        self.invocation.caps()
    }

    /// Facts already committed for this attempt and therefore reusable after recovery.
    pub fn recorded_facts(&self) -> &RecordedFacts {
        self.invocation.recorded_facts()
    }

    /// Store-owned projection snapshot observed before the runner invocation.
    pub fn projections(&self) -> &store::ProjectionSnapshot {
        self.invocation.projections()
    }

    pub(crate) fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.invocation.runtime_spec()
    }

    pub(crate) fn run_stream(&self) -> &'a [store::KernelEventEnvelope] {
        self.invocation.run_stream()
    }
}

/// Facts committed for one node attempt before recovery resumed execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordedFacts {
    pub(crate) facts: BTreeMap<events::FactKey, RecordedFact>,
}

impl RecordedFacts {
    /// Returns true when no facts have been recorded for the attempt.
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Returns a recorded fact by stable fact key.
    pub fn get(&self, fact_key: &events::FactKey) -> Option<&RecordedFact> {
        self.facts.get(fact_key)
    }

    /// Iterates recorded facts in deterministic fact-key order.
    pub fn iter(&self) -> impl Iterator<Item = (&events::FactKey, &RecordedFact)> {
        self.facts.iter()
    }
}

/// Store-projected read fact available for same-attempt recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFact {
    /// Stable fact key.
    pub fact_key: events::FactKey,
    /// Request schema id.
    pub request_schema_id: SchemaId,
    /// Canonical request hash.
    pub request_hash: ContentDigest,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Canonical response hash.
    pub response_hash: ContentDigest,
    /// Response artifact id.
    pub artifact_id: ArtifactId,
    /// Capability kind used for the original read.
    pub capability_kind: CapabilityKind,
    /// Capability version used for the original read.
    pub capability_version: CapabilityVersion,
    /// Adapter kind used for the original read.
    pub adapter_kind: AdapterKind,
    /// Adapter version used for the original read.
    pub adapter_version: AdapterVersion,
}

/// Runtime capabilities minted for a node attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedRuntimeCapabilities {
    pub(crate) node_id: NodeId,
    pub(crate) descriptor: CapabilitySetDescriptor,
}

impl CertifiedRuntimeCapabilities {
    pub(crate) fn new(node_id: NodeId, descriptor: CapabilitySetDescriptor) -> Self {
        Self {
            node_id,
            descriptor,
        }
    }

    /// Node id these capabilities were minted for.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Certified capability descriptors.
    pub fn descriptors(&self) -> &[CapabilityDescriptor] {
        &self.descriptor.capabilities
    }

    /// Returns true when the certified capability set contains this kind/version.
    pub fn contains(&self, kind: &CapabilityKind, version: &CapabilityVersion) -> bool {
        self.descriptor
            .capabilities
            .iter()
            .any(|capability| capability.kind == *kind && capability.version == *version)
    }
}

/// Materialized typed input tree for one node attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedInputs {
    /// Certified input schema id.
    pub input_schema_id: SchemaId,
    /// Root materialized input node.
    pub root: MaterializedInputNode,
}

/// Materialized input tree node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedInputNode {
    /// Unit input.
    Unit,
    /// Certified cell input.
    Cell(Box<MaterializedCell>),
    /// Tuple input.
    Tuple(Vec<MaterializedInputNode>),
    /// Struct input.
    Struct(Vec<NamedMaterializedInput>),
    /// Vector input.
    Vec(Vec<MaterializedInputNode>),
    /// Non-empty vector input.
    NonEmptyVec(Vec<MaterializedInputNode>),
}

/// Named materialized struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedMaterializedInput {
    /// Certified field path.
    pub field_path: spec::PublicFieldPath,
    /// Materialized field node.
    pub node: MaterializedInputNode,
}

/// Materialized certified cell evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedCell {
    /// Cell id.
    pub cell_id: CellId,
    /// Schema id.
    pub schema_id: SchemaId,
    /// Semantic type id.
    pub semantic_type_id: mfm_ids::SemanticTypeId,
    /// Value lineage ref.
    pub value_lineage: spec::ValueLineageRef,
    /// Terminal evidence.
    pub terminal: MaterializedCellTerminal,
}

/// Materialized terminal cell evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedCellTerminal {
    /// Seed material from `RunStarted`.
    Seed {
        /// Seed id.
        seed_id: mfm_ids::SeedId,
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
    },
    /// Produced node output.
    Produced {
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
    },
    /// Skipped node output.
    Skipped {
        /// Skip reason.
        skip_reason: events::SkipReason,
    },
}
