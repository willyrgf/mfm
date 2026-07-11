use std::collections::BTreeMap;

use mfm_capabilities::{CapabilityDescriptor, CapabilitySetDescriptor};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, NodeId, RunId, SchemaId, SpecHash,
};
use mfm_program::{CertifiedContext, StateContext};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::history::{
    committed_config_artifact, materialize_inputs, recorded_facts_for_attempt, RuntimeRunView,
};
use crate::{CertifiedRuntimeSpec, Result};

/// Certified transition context authority for one runner invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedInvocationContext {
    spec: Option<spec::CertifiedContextSpec>,
}

impl CertifiedInvocationContext {
    pub(crate) fn for_node(
        runtime_spec: &CertifiedRuntimeSpec,
        node: &spec::NodeSpec,
    ) -> Result<Self> {
        match &node.context {
            spec::NodeContextSpec::NoContext => Ok(Self { spec: None }),
            spec::NodeContextSpec::Required { context_ref } => {
                let context = runtime_spec.context(context_ref).ok_or_else(|| {
                    crate::RuntimeError::InvalidSpec(format!(
                        "node {} requires missing certified context {}",
                        node.node_id, context_ref
                    ))
                })?;
                Ok(Self {
                    spec: Some(context.clone()),
                })
            }
        }
    }

    /// Materializes state context authority for either no-context or typed-context states.
    pub fn certified_context<C>(&self) -> Result<CertifiedContext<C>>
    where
        C: StateContext,
    {
        C::materialize_certified(self.spec.as_ref())
            .map_err(|error| crate::RuntimeError::InvalidRunnerOutput(error.to_string()))
    }
}

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
    pub(crate) context: CertifiedInvocationContext,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) attempt_no: u32,
    pub(crate) config_artifact: store::ArtifactEvidenceRef,
    pub(crate) inputs: MaterializedInputs,
    pub(crate) caps: CertifiedRuntimeCapabilities,
    pub(crate) recorded_facts: RecordedFacts,
    pub(crate) projections: &'a store::ProjectionSnapshot,
    pub(crate) run_stream: &'a [store::KernelEventEnvelope],
    pub(crate) view: &'a RuntimeRunView,
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

    /// Certified semantic transition context for this invocation.
    pub fn context(&self) -> &CertifiedInvocationContext {
        &self.context
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

    pub(crate) fn artifact_byte_authority(&self) -> &'a store::ArtifactByteAuthorityMap {
        &self.view.artifact_byte_authority
    }
}

/// Builder for sealed runner invocation authority.
pub(crate) struct InvocationBuilder<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
}

/// Certified inputs needed to construct a sealed runner invocation.
pub(crate) struct InvocationBuilderInput<'a> {
    /// Runtime authority wrapper for the certified spec.
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    /// Run id being executed.
    pub(crate) run_id: &'a RunId,
    /// Certified node being invoked.
    pub(crate) node: &'a spec::NodeSpec,
    /// Certified state descriptor for the node.
    pub(crate) descriptor: &'a spec::StateDescriptorIdentity,
    /// Certified output cell for the node.
    pub(crate) output_cell: &'a spec::CellSpec,
    /// Store-owned attempt id.
    pub(crate) attempt_id: &'a AttemptId,
    /// Attempt number for this node.
    pub(crate) attempt_no: u32,
    /// Verified latest run view used for materialization.
    pub(crate) view: &'a RuntimeRunView,
}

struct InvocationMaterial {
    config_artifact: store::ArtifactEvidenceRef,
    inputs: MaterializedInputs,
    caps: CertifiedRuntimeCapabilities,
    recorded_facts: RecordedFacts,
    context: CertifiedInvocationContext,
}

impl<'a> InvocationBuilder<'a> {
    /// Creates an invocation builder for one certified node attempt.
    pub(crate) fn new(input: InvocationBuilderInput<'a>) -> Self {
        let InvocationBuilderInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no,
            view,
        } = input;
        Self {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no,
            view,
        }
    }

    fn materialize(&self) -> Result<InvocationMaterial> {
        let config_artifact = committed_config_artifact(self.node, self.view)?;
        let inputs = materialize_inputs(self.runtime_spec, self.node, self.view)?;
        let caps = CertifiedRuntimeCapabilities::for_node(self.node);
        let recorded_facts = recorded_facts_for_attempt(
            &self.view.projections,
            &self.node.node_id,
            self.attempt_id,
        )?;
        let context = self.runtime_spec.invocation_context_for_node(self.node)?;
        Ok(InvocationMaterial {
            config_artifact,
            inputs,
            caps,
            recorded_facts,
            context,
        })
    }

    /// Builds the sealed invocation from certified spec and verified run-stream state.
    pub(crate) fn build(self) -> Result<PreparedRunnerInvocation<'a>> {
        let material = self.materialize()?;
        Ok(PreparedRunnerInvocation {
            runtime_spec: self.runtime_spec,
            run_id: self.run_id,
            spec_hash: self.runtime_spec.spec_hash(),
            node: self.node,
            descriptor: self.descriptor,
            output_cell: self.output_cell,
            context: material.context,
            attempt_id: self.attempt_id,
            attempt_no: self.attempt_no,
            config_artifact: material.config_artifact,
            inputs: material.inputs,
            caps: material.caps,
            recorded_facts: material.recorded_facts,
            projections: &self.view.projections,
            run_stream: &self.view.stream,
            view: self.view,
        })
    }

    /// Builds pure pre-invocation context for resource-lane preflight only.
    pub(crate) fn build_pre_invocation(self) -> Result<PreInvocationRunCtx<'a>> {
        let material = self.materialize()?;
        Ok(PreInvocationRunCtx {
            runtime_spec: self.runtime_spec,
            run_id: self.run_id,
            spec_hash: self.runtime_spec.spec_hash(),
            node: self.node,
            descriptor: self.descriptor,
            output_cell: self.output_cell,
            context: material.context,
            attempt_id: self.attempt_id,
            attempt_no: self.attempt_no,
            config_artifact: material.config_artifact,
            inputs: material.inputs,
            caps: material.caps,
            recorded_facts: material.recorded_facts,
            projections: &self.view.projections,
        })
    }
}

/// Pure pre-invocation context supplied only to resource-lane preflight hooks.
///
/// This context is built before [`PreparedRunnerInvocation`] and before runner execution. It
/// exposes certified config/input/fact evidence needed to resolve lane authority, but it is not a
/// live invocation boundary and must not be used for transport, signing, clock, or other ambient IO.
pub struct PreInvocationRunCtx<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) spec_hash: &'a SpecHash,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) descriptor: &'a spec::StateDescriptorIdentity,
    pub(crate) output_cell: &'a spec::CellSpec,
    pub(crate) context: CertifiedInvocationContext,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) attempt_no: u32,
    pub(crate) config_artifact: store::ArtifactEvidenceRef,
    pub(crate) inputs: MaterializedInputs,
    pub(crate) caps: CertifiedRuntimeCapabilities,
    pub(crate) recorded_facts: RecordedFacts,
    pub(crate) projections: &'a store::ProjectionSnapshot,
}

impl<'a> PreInvocationRunCtx<'a> {
    /// Run id being preflighted.
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

    /// Certified semantic transition context for this pre-invocation hook.
    pub fn context(&self) -> &CertifiedInvocationContext {
        &self.context
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

    /// Store-owned projection snapshot observed before invocation construction.
    pub fn projections(&self) -> &store::ProjectionSnapshot {
        self.projections
    }

    pub(crate) fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.runtime_spec
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

    /// Returns another certified node from the same runtime spec.
    pub fn certified_node(&self, node_id: &NodeId) -> Option<&'a spec::NodeSpec> {
        self.invocation.runtime_spec().node(node_id)
    }

    /// Returns certified transition-context authority for another node in the same runtime spec.
    pub fn invocation_context_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<CertifiedInvocationContext> {
        self.invocation
            .runtime_spec()
            .invocation_context_for_node(node)
    }

    /// Certified state descriptor identity for the node.
    pub fn descriptor(&self) -> &'a spec::StateDescriptorIdentity {
        self.invocation.descriptor()
    }

    /// Certified output cell spec for the node.
    pub fn output_cell(&self) -> &'a spec::CellSpec {
        self.invocation.output_cell()
    }

    /// Certified semantic transition context for this runner invocation.
    pub fn context(&self) -> &CertifiedInvocationContext {
        self.invocation.context()
    }

    /// Materializes certified context authority for this runner invocation.
    pub fn certified_context<C>(&self) -> Result<CertifiedContext<C>>
    where
        C: StateContext,
    {
        self.context().certified_context::<C>()
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

    /// Materializes certified inputs for another node against the same verified run stream.
    pub fn materialize_node_inputs(&self, node: &spec::NodeSpec) -> Result<MaterializedInputs> {
        materialize_inputs(self.invocation.runtime_spec(), node, self.invocation.view)
    }

    pub(crate) fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.invocation.runtime_spec()
    }

    pub(crate) fn run_stream(&self) -> &'a [store::KernelEventEnvelope] {
        self.invocation.run_stream()
    }

    pub(crate) fn artifact_byte_authority(&self) -> &'a store::ArtifactByteAuthorityMap {
        self.invocation.artifact_byte_authority()
    }
}

/// Facts committed for one node attempt before recovery resumed execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordedFacts {
    pub(crate) facts: BTreeMap<mfm_facts::FactClaimId, RecordedFact>,
}

impl RecordedFacts {
    /// Returns true when no facts have been recorded for the attempt.
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Returns a recorded fact by stable claim id.
    pub fn get(&self, claim_id: &mfm_facts::FactClaimId) -> Option<&RecordedFact> {
        self.facts.get(claim_id)
    }

    /// Iterates recorded facts in deterministic claim-id order.
    pub fn iter(&self) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &RecordedFact)> {
        self.facts.iter()
    }

    /// Iterates recorded facts for a subject key in deterministic claim-id order.
    pub fn by_fact_key<'a>(
        &'a self,
        fact_key: &'a mfm_facts::FactKey,
    ) -> impl Iterator<Item = (&'a mfm_facts::FactClaimId, &'a RecordedFact)> + 'a {
        self.facts
            .iter()
            .filter(move |(_, fact)| &fact.fact_key == fact_key)
    }
}

/// Store-projected read fact available for same-attempt recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFact {
    /// Store-derived identity for the recorded claim.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Subject grouping key; not a unique claim identity.
    pub fact_key: mfm_facts::FactKey,
    /// Request schema id, when request evidence is present.
    pub request_schema_id: Option<SchemaId>,
    /// Canonical request hash, when request evidence is present.
    pub request_hash: Option<ContentDigest>,
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
    pub(crate) fn for_node(node: &spec::NodeSpec) -> Self {
        Self {
            node_id: node.node_id.clone(),
            descriptor: node.capability_bindings.clone(),
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
    /// Certified context constraint carried by the cell.
    pub context: spec::CellContextSpec,
    /// Terminal evidence.
    pub terminal: MaterializedCellTerminal,
}

/// Materialized terminal cell evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedCellTerminal {
    /// Seed material from `RunAdmitted`.
    Seed {
        /// Seed id.
        seed_id: mfm_ids::SeedId,
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
        /// Exact retained-artifact evidence identity.
        evidence_hash: ContentDigest,
    },
    /// Produced node output.
    Produced {
        /// Producer node id.
        producer_node_id: NodeId,
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
        /// Exact retained-artifact evidence identity.
        evidence_hash: ContentDigest,
    },
    /// Skipped node output.
    Skipped {
        /// Skip reason.
        skip_reason: events::SkipReason,
    },
}
