#![warn(missing_docs)]
//! Serial typed scheduler for certified MFM execution specs.
//!
//! This crate owns the first certified runtime boundary. It derives runnable nodes, materialized
//! input evidence, runner bindings, and runtime capabilities only from a verified
//! [`mfm_spec::v1::CertifiedSpecEnvelope`] plus store-owned typed projections.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilityDescriptor, CapabilitySetDescriptor};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, NodeId, RunId, SchemaId, SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

/// Result type for typed runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Boxed future returned by an erased typed runner.
pub type ErasedRunnerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedRunnerOutput>> + Send + 'a>>;

/// Object-safe erased runner boundary used after typed spec certification.
///
/// Runner selection is keyed by the certified node descriptor id. The runner receives only
/// store-verified input cell evidence and certified capability descriptors.
pub trait ErasedNodeRunner: Send + Sync {
    /// Executes one certified node attempt.
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a>;
}

/// Context supplied to an erased node runner.
pub struct ErasedRunCtx<'a> {
    /// Run id being executed.
    pub run_id: &'a RunId,
    /// Certified typed spec hash.
    pub spec_hash: &'a SpecHash,
    /// Certified node spec.
    pub node: &'a spec::NodeSpec,
    /// Certified state descriptor identity for the node.
    pub descriptor: &'a spec::StateDescriptorIdentity,
    /// Certified output cell spec for the node.
    pub output_cell: &'a spec::CellSpec,
    /// Store-owned attempt id minted by the scheduler.
    pub attempt_id: &'a AttemptId,
    /// Attempt number for this node.
    pub attempt_no: u32,
    /// Materialized input evidence derived only from certified cells.
    pub inputs: &'a MaterializedInputs,
    /// Runtime capabilities minted only from the certified node capability set.
    pub caps: &'a CertifiedRuntimeCapabilities,
    /// Facts already committed for this attempt and therefore reusable after recovery.
    pub recorded_facts: &'a RecordedFacts,
    /// Store-owned projection snapshot observed before the attempt.
    pub projections: &'a store::ProjectionSnapshot,
}

/// Facts committed for one node attempt before recovery resumed execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordedFacts {
    facts: BTreeMap<events::FactKey, RecordedFact>,
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

/// Typed payload batch returned by an erased runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedRunnerOutput {
    /// Artifact evidence written before payload commit.
    pub required_artifacts: Vec<store::ArtifactEvidenceRef>,
    /// Typed event payloads to commit atomically for this attempt.
    pub payloads: Vec<events::KernelEventPayload>,
}

impl ErasedRunnerOutput {
    /// Creates an output batch from payloads with no additional artifact evidence.
    pub fn new(payloads: Vec<events::KernelEventPayload>) -> Self {
        Self {
            required_artifacts: Vec::new(),
            payloads,
        }
    }
}

/// Runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Certified spec hash verification failed.
    SpecHash(String),
    /// Certified spec structure is not executable by the runtime.
    InvalidSpec(String),
    /// The run stream does not match the certified spec or requested run id.
    InvalidRunStream(String),
    /// A runner binding is missing or inconsistent with certified descriptor evidence.
    RunnerBinding(String),
    /// No node is runnable and the run is not complete.
    Blocked(String),
    /// Input materialization failed.
    InputMaterialization(String),
    /// Runner output violated certified node or capability authority.
    InvalidRunnerOutput(String),
    /// Store contract rejected a typed commit.
    Store(String),
    /// Identity construction failed.
    Identity(String),
    /// Canonical JSON construction failed.
    Canonical(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpecHash(message) => write!(f, "certified spec hash error: {message}"),
            Self::InvalidSpec(message) => write!(f, "invalid typed runtime spec: {message}"),
            Self::InvalidRunStream(message) => write!(f, "invalid typed run stream: {message}"),
            Self::RunnerBinding(message) => write!(f, "typed runner binding error: {message}"),
            Self::Blocked(message) => write!(f, "typed scheduler blocked: {message}"),
            Self::InputMaterialization(message) => {
                write!(f, "typed input materialization failed: {message}")
            }
            Self::InvalidRunnerOutput(message) => write!(f, "invalid runner output: {message}"),
            Self::Store(message) => write!(f, "typed store error: {message}"),
            Self::Identity(message) => write!(f, "identity error: {message}"),
            Self::Canonical(message) => write!(f, "canonical JSON error: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<store::StoreError> for RuntimeError {
    fn from(error: store::StoreError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<mfm_ids::IdentityError> for RuntimeError {
    fn from(error: mfm_ids::IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_spec::SpecError> for RuntimeError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::SpecHash(error.to_string())
    }
}

/// Certified executable runtime spec with indexes used by the serial scheduler.
#[derive(Debug, Clone)]
pub struct CertifiedRuntimeSpec {
    envelope: spec::CertifiedSpecEnvelope,
    state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    nodes: BTreeMap<NodeId, spec::NodeSpec>,
    cells: BTreeMap<CellId, spec::CellSpec>,
    topological_order: Vec<NodeId>,
}

impl CertifiedRuntimeSpec {
    /// Verifies a certified spec envelope and builds deterministic runtime indexes.
    pub fn new(envelope: spec::CertifiedSpecEnvelope) -> Result<Self> {
        envelope.verify_hash()?;
        let mut state_descriptors = BTreeMap::new();
        for descriptor in &envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                let previous = state_descriptors
                    .insert(identity.descriptor_id.clone(), identity.as_ref().clone());
                if previous.is_some() {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "duplicate state descriptor {}",
                        identity.descriptor_id
                    )));
                }
            }
        }

        let mut cells = BTreeMap::new();
        for cell in &envelope.spec.cells {
            if cells.insert(cell.cell_id.clone(), cell.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate cell {}",
                    cell.cell_id
                )));
            }
        }

        let mut nodes = BTreeMap::new();
        for node in &envelope.spec.nodes {
            if nodes.insert(node.node_id.clone(), node.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate node {}",
                    node.node_id
                )));
            }
        }

        let runtime = Self {
            envelope,
            state_descriptors,
            nodes,
            cells,
            topological_order: Vec::new(),
        };
        runtime.validate_runtime_contract()?;
        let topological_order = runtime.compute_topological_order()?;
        Ok(Self {
            topological_order,
            ..runtime
        })
    }

    /// Returns the certified spec envelope.
    pub fn envelope(&self) -> &spec::CertifiedSpecEnvelope {
        &self.envelope
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.envelope.spec_hash
    }

    /// Returns the hash-defining typed execution spec.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        &self.envelope.spec
    }

    /// Returns deterministic topological node ids.
    pub fn topological_order(&self) -> &[NodeId] {
        &self.topological_order
    }

    /// Returns a certified node by id.
    pub fn node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.nodes.get(node_id)
    }

    /// Returns a certified cell by id.
    pub fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        self.cells.get(cell_id)
    }

    /// Returns the certified state descriptor for a node.
    pub fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&spec::StateDescriptorIdentity> {
        self.state_descriptors
            .get(&node.descriptor_id)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} references missing state descriptor {}",
                    node.node_id, node.descriptor_id
                ))
            })
    }

    fn validate_runtime_contract(&self) -> Result<()> {
        let config_refs = self
            .spec()
            .config_refs
            .iter()
            .map(config_ref_key)
            .collect::<BTreeSet<_>>();

        for seed in &self.spec().seeds {
            let cell = self.cells.get(&seed.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ))
            })?;
            if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
                || cell.scope_id != seed.scope_id
                || cell.schema_id != seed.schema_id
                || cell.semantic_type_id != seed.semantic_type_id
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "seed {} cell metadata does not match certified cell {}",
                    seed.seed_id, seed.cell_id
                )));
            }
        }

        for node in self.nodes.values() {
            let descriptor = self.state_descriptor_for_node(node)?;
            if descriptor.state_kind != node.state_kind
                || descriptor.state_version != node.state_version
                || descriptor.config_schema_id != node.config_ref.schema_id
                || descriptor.input_schema_id != node.input_bindings.input_schema_id
                || descriptor.output_schema_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.schema_id.clone())
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "node {} output cell {} is missing",
                                node.node_id, node.output_cell
                            ))
                        })?
                || descriptor.output_semantic_type_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.semantic_type_id.clone())
                        .expect("checked above")
                || descriptor.effect_kind != node.effect_kind
                || descriptor.capabilities != node.capability_bindings
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} does not match certified state descriptor {}",
                    node.node_id, node.descriptor_id
                )));
            }
            match (&node.side_effect, &descriptor.side_effect_contract_digest) {
                (Some(contract), Some(descriptor_digest))
                    if descriptor_digest == &contract.contract_digest => {}
                (Some(_), _) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "side-effect node {} lacks matching descriptor side-effect contract",
                        node.node_id
                    )));
                }
                (None, Some(_)) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "non-side-effect node {} references a descriptor with side-effect contract",
                        node.node_id
                    )));
                }
                (None, None) => {}
            }

            if !config_refs.contains(&config_ref_key(&node.config_ref)) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} references missing config artifact {}",
                    node.node_id, node.config_ref.artifact_id
                )));
            }

            let output = self.cells.get(&node.output_cell).expect("checked above");
            if output.producer != spec::CellProducer::Node(node.node_id.clone()) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} output cell {} has mismatched producer",
                    node.node_id, node.output_cell
                )));
            }

            let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
            let expected_predecessors = self.predecessors_for_input_cells(&input_cells)?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} deterministic predecessors do not match input cells",
                    node.node_id
                )));
            }
        }

        for output in &self.spec().public_outputs.outputs {
            let cell = self.cells.get(&output.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public output {} references missing cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                ))
            })?;
            if cell.producer != output.producer
                || cell.scope_id != output.scope_id
                || cell.semantic_type_id != output.semantic_type_id
                || cell.schema_id != output.schema_id
                || cell.value_lineage != output.value_lineage
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "public output {} does not match certified cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                )));
            }
        }

        Ok(())
    }

    fn validate_input_binding(&self, input: &spec::InputBindingNodeSpec) -> Result<Vec<CellId>> {
        let mut cells = Vec::new();
        self.validate_input_node(input, &mut cells)?;
        Ok(cells)
    }

    fn validate_input_node(
        &self,
        input: &spec::InputBindingNodeSpec,
        cells: &mut Vec<CellId>,
    ) -> Result<()> {
        match input {
            spec::InputBindingNodeSpec::Unit => {}
            spec::InputBindingNodeSpec::Cell(cell) => {
                let certified = self.cells.get(&cell.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "input binding references missing cell {}",
                        cell.cell_id
                    ))
                })?;
                if certified.semantic_type_id != cell.semantic_type_id
                    || certified.schema_id != cell.schema_id
                    || certified.value_lineage != cell.value_lineage
                {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "input binding for cell {} does not match certified cell metadata",
                        cell.cell_id
                    )));
                }
                cells.push(cell.cell_id.clone());
            }
            spec::InputBindingNodeSpec::Tuple(elements) => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Struct(fields) => {
                for field in fields {
                    self.validate_input_node(&field.node, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Vec { elements, .. }
            | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
        }
        Ok(())
    }

    fn predecessors_for_input_cells(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            let cell = self.cells.get(cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!("input cell {cell_id} is missing"))
            })?;
            if let spec::CellProducer::Node(node_id) = &cell.producer {
                predecessors.insert(node_id.clone());
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn compute_topological_order(&self) -> Result<Vec<NodeId>> {
        let mut indegree = BTreeMap::<NodeId, usize>::new();
        let mut successors = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
        for node_id in self.nodes.keys() {
            indegree.insert(node_id.clone(), 0);
            successors.insert(node_id.clone(), BTreeSet::new());
        }
        for node in self.nodes.values() {
            for predecessor in &node.deterministic_predecessors {
                if !self.nodes.contains_key(predecessor) {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "node {} references missing predecessor {}",
                        node.node_id, predecessor
                    )));
                }
                successors
                    .get_mut(predecessor)
                    .expect("predecessor exists")
                    .insert(node.node_id.clone());
                *indegree.get_mut(&node.node_id).expect("node exists") += 1;
            }
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(node_id, count)| (*count == 0).then_some(node_id.clone()))
            .collect::<VecDeque<_>>();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node_id) = ready.pop_front() {
            order.push(node_id.clone());
            for successor in successors.get(&node_id).expect("successors exist") {
                let count = indegree.get_mut(successor).expect("successor exists");
                *count -= 1;
                if *count == 0 {
                    let index = ready
                        .iter()
                        .position(|queued| successor < queued)
                        .unwrap_or(ready.len());
                    ready.insert(index, successor.clone());
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(RuntimeError::InvalidSpec(
                "certified node graph contains a cycle".to_owned(),
            ));
        }
        Ok(order)
    }
}

/// Registered erased runner binding for one certified state descriptor.
#[derive(Clone)]
pub struct ErasedRunnerBinding {
    descriptor_id: DescriptorId,
    factory_id: events::RunnerFactoryId,
    executable: events::ExecutableIdentity,
    runner: Arc<dyn ErasedNodeRunner>,
}

impl ErasedRunnerBinding {
    /// Creates a runner binding for a certified state descriptor id.
    pub fn new(
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<Self> {
        if executable.factory_id != factory_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "executable factory {} does not match binding factory {}",
                executable.factory_id, factory_id
            )));
        }
        Ok(Self {
            descriptor_id,
            factory_id,
            executable,
            runner,
        })
    }

    /// Certified descriptor id this binding executes.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }

    /// Runner factory id.
    pub fn factory_id(&self) -> &events::RunnerFactoryId {
        &self.factory_id
    }

    /// Executable identity for run-start evidence.
    pub fn executable(&self) -> &events::ExecutableIdentity {
        &self.executable
    }
}

/// Registry of erased runners keyed by certified state descriptor id.
#[derive(Clone, Default)]
pub struct ErasedRunnerRegistry {
    bindings: BTreeMap<DescriptorId, ErasedRunnerBinding>,
}

impl ErasedRunnerRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one erased runner binding.
    pub fn register(&mut self, binding: ErasedRunnerBinding) -> Result<()> {
        if self
            .bindings
            .insert(binding.descriptor_id.clone(), binding)
            .is_some()
        {
            return Err(RuntimeError::RunnerBinding(
                "duplicate runner binding".to_owned(),
            ));
        }
        Ok(())
    }

    fn resolve(
        &self,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<&ErasedRunnerBinding> {
        let binding = self.bindings.get(&node.descriptor_id).ok_or_else(|| {
            RuntimeError::RunnerBinding(format!(
                "missing runner binding for node {} descriptor {}",
                node.node_id, node.descriptor_id
            ))
        })?;
        if binding.descriptor_id != node.descriptor_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding descriptor mismatch for node {}",
                node.node_id
            )));
        }
        if binding.factory_id.as_str() != descriptor.runner {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding factory {} does not match descriptor runner {} for node {}",
                binding.factory_id, descriptor.runner, node.node_id
            )));
        }
        Ok(binding)
    }

    fn executables_for_spec(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
    ) -> Result<Vec<events::ExecutableIdentity>> {
        let mut seen = BTreeSet::new();
        let mut executables = Vec::new();
        for node_id in runtime_spec.topological_order() {
            let node = runtime_spec.node(node_id).expect("topological node exists");
            let descriptor = runtime_spec.state_descriptor_for_node(node)?;
            let binding = self.resolve(node, descriptor)?;
            let key = binding.executable.factory_id.as_str().to_owned();
            if seen.insert(key) {
                executables.push(binding.executable.clone());
            }
        }
        Ok(executables)
    }
}

/// Runtime capabilities minted for a node attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedRuntimeCapabilities {
    node_id: NodeId,
    descriptor: CapabilitySetDescriptor,
}

impl CertifiedRuntimeCapabilities {
    fn new(node_id: NodeId, descriptor: CapabilitySetDescriptor) -> Self {
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

#[derive(Debug, Clone)]
struct RuntimeRunView {
    projections: store::ProjectionSnapshot,
    seed_cells: BTreeMap<CellId, events::SeedCellRef>,
}

impl RuntimeRunView {
    fn from_store<S: store::TypedRunEventStore + ?Sized>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self> {
        let stream = store.load_run_stream(run_id);
        store::ProjectionSnapshot::validate_run_stream(&stream)?;
        let projections = store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
        let mut run_started = None;
        for event in &stream {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    if &payload.run_id != run_id {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "run stream contains RunStarted for {} while executing {}",
                            payload.run_id, run_id
                        )));
                    }
                    if &payload.spec_hash != runtime_spec.spec_hash() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "RunStarted spec hash {} does not match certified {}",
                            payload.spec_hash,
                            runtime_spec.spec_hash()
                        )));
                    }
                    if run_started.replace(payload.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(
                            "run stream contains multiple RunStarted events".to_owned(),
                        ));
                    }
                }
                payload if payload_spec_hash(payload) != *runtime_spec.spec_hash() => {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "event payload spec hash {} does not match certified {}",
                        payload_spec_hash(payload),
                        runtime_spec.spec_hash()
                    )));
                }
                _ => {}
            }
        }
        let Some(run_started) = run_started else {
            return Err(RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            ));
        };
        validate_historical_run_stream(runtime_spec, &stream, &projections)?;
        let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
        Ok(Self {
            projections,
            seed_cells,
        })
    }
}

/// Evidence needed to append `RunStarted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartEvidence {
    /// Artifact evidence containing the certified spec bytes.
    pub spec_artifact: store::ArtifactEvidenceRef,
    /// Artifact evidence for every certified config reference.
    pub config_artifacts: Vec<store::ArtifactEvidenceRef>,
    /// Framework build/version identity.
    pub framework_version: events::FrameworkVersion,
    /// Source revision identity.
    pub source_revision: events::SourceRevision,
    /// Adapter executable identities bound to the run.
    pub adapter_executables: Vec<events::ExecutableIdentity>,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<events::SeedCellRef>,
}

/// Result of one serial scheduler drive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// At least one node attempt ran and committed.
    Advanced,
    /// No node is currently runnable.
    Blocked,
    /// Public output has already been projected.
    PublicOutputProjected,
}

struct RunnableNode<'a> {
    node: &'a spec::NodeSpec,
    attempt: AttemptPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttemptPlan {
    StartNew,
    Continue {
        attempt_id: AttemptId,
        attempt_no: u32,
    },
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runners: ErasedRunnerRegistry,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry.
    pub fn new(runners: ErasedRunnerRegistry) -> Self {
        Self { runners }
    }

    /// Appends the typed `RunStarted` event after validating seed and runner executable evidence.
    pub fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunStartEvidence,
    ) -> Result<store::CommitOutcome> {
        let spec_artifact = validate_spec_artifact(runtime_spec, evidence.spec_artifact)?;
        let config_artifacts = validate_config_artifacts(runtime_spec, evidence.config_artifacts)?;
        let seed_cells = validate_seed_cells(runtime_spec, &evidence.seed_cells)?;
        let runner_executables = self.runners.executables_for_spec(runtime_spec)?;
        store.record_artifact_evidence(spec_artifact.clone())?;
        for artifact in &config_artifacts {
            store.record_artifact_evidence(artifact.clone())?;
        }
        for seed in seed_cells.values() {
            store.record_artifact_evidence(store_seed_artifact(seed))?;
        }
        let mut required_artifacts =
            Vec::with_capacity(1 + config_artifacts.len() + seed_cells.len());
        required_artifacts.push(spec_artifact.clone());
        required_artifacts.extend(config_artifacts);
        required_artifacts.extend(seed_cells.values().map(store_seed_artifact));
        let payload = events::KernelEventPayload::RunStarted(events::RunStarted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            spec_artifact_id: spec_artifact.artifact_id,
            spec_media_type: runtime_spec.spec().media_type.clone(),
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables,
            adapter_executables: evidence.adapter_executables,
            canonicalizer_identity: runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            framework_version: evidence.framework_version,
            source_revision: evidence.source_revision,
            seed_cells: evidence.seed_cells,
        });
        let request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: store::CommitKey::new(format!(
                "run-start:{}",
                runtime_spec.spec_hash().as_str()
            ))?,
            payloads: vec![payload],
            required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                ..store::CommitPreconditions::default()
            },
        };
        Ok(store.append_typed_run_commit(request)?)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        if view
            .projections
            .public_output(&runtime_spec.spec().public_outputs.public_schema_id)
            .is_some()
        {
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        let Some(runnable) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
            .await?;
        Ok(SchedulerStatus::Advanced)
    }

    /// Runs deterministic runnable nodes until no node is runnable or public output is projected.
    pub async fn drive_until_blocked<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        let inputs = materialize_inputs(runtime_spec, node, view)?;
        let caps = CertifiedRuntimeCapabilities::new(
            node.node_id.clone(),
            node.capability_bindings.clone(),
        );
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew => {
                let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                let start_payload =
                    events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                        spec_hash: runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        attempt_no,
                        state_kind: node.state_kind.clone(),
                        state_version: node.state_version.clone(),
                    });
                let mut preconditions = store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                };
                preconditions
                    .required_cell_states
                    .extend(node_cell_preconditions(runtime_spec, node)?);
                let start_request = store::TypedCommitRequest {
                    run_id: run_id.clone(),
                    expected_next_seq: store.expected_next_seq(run_id),
                    commit_key: store::CommitKey::new(format!(
                        "attempt-start:{}:{}",
                        node.node_id, attempt_id
                    ))?,
                    payloads: vec![start_payload],
                    required_artifacts: Vec::new(),
                    preconditions,
                };
                store.append_typed_run_commit(start_request)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_projection = store.projection_snapshot().clone();
        let recorded_facts =
            recorded_facts_for_attempt(&latest_projection, &node.node_id, &attempt_id)?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx {
                run_id,
                spec_hash: runtime_spec.spec_hash(),
                node,
                descriptor,
                output_cell,
                attempt_id: &attempt_id,
                attempt_no,
                inputs: &inputs,
                caps: &caps,
                recorded_facts: &recorded_facts,
                projections: &latest_projection,
            })
            .await?;
        validate_runner_output(
            runtime_spec,
            node,
            &attempt_id,
            &caps,
            &recorded_facts,
            &latest_projection,
            &output,
        )?;
        for artifact in &output.required_artifacts {
            store.record_artifact_evidence(artifact.clone())?;
        }
        let preconditions =
            runner_output_preconditions(node, &attempt_id, &latest_projection, &output.payloads)?;
        let terminal_request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: runner_output_commit_key(node, &attempt_id, &output.payloads)?,
            payloads: output.payloads,
            required_artifacts: output.required_artifacts,
            preconditions,
        };
        store.append_typed_run_commit(terminal_request)?;
        Ok(())
    }
}

fn runner_output_commit_key(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitKey> {
    let mut fragments = BTreeSet::new();
    for payload in payloads {
        fragments.insert(runner_output_commit_fragment(payload));
    }
    if fragments.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let suffix = fragments.into_iter().collect::<Vec<_>>().join("+");
    Ok(store::CommitKey::new(format!(
        "attempt-output:{}:{}:{}",
        node.node_id, attempt_id, suffix
    ))?)
}

fn runner_output_commit_fragment(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("completed:{}", payload.output_cell_id)
        }
        events::KernelEventPayload::StateAttemptFailed(_) => "failed".to_owned(),
        events::KernelEventPayload::CellProduced(payload) => {
            format!("cell-produced:{}", payload.cell_id)
        }
        events::KernelEventPayload::CellSkipped(payload) => {
            format!("cell-skipped:{}", payload.cell_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => format!("fact:{}", payload.fact_key),
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}", payload.artifact_ref.artifact_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public-output:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!("public-output-failed:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!("sidefx-intent:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::RunStarted(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::RetentionRefsAppended(_)
        | events::KernelEventPayload::RetentionManifestProjected(_)
        | events::KernelEventPayload::StateAttemptStarted(_) => "scheduler-owned".to_owned(),
    }
}

fn runner_output_preconditions(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitPreconditions> {
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
            "attempt:{}:{}",
            node.node_id, attempt_id
        ))?],
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        required_public_output_absent: matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ),
        ..store::CommitPreconditions::default()
    };

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let mut requires_terminal_confirmation = false;
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.ledger_key.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.ledger_key) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            ledger_key: payload.ledger_key.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => {
                requires_terminal_confirmation = true;
            }
            _ => {}
        }
    }

    if requires_terminal_confirmation {
        let projection = side_effect_projection_for_attempt(projections, node, attempt_id)?
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempted output without ledger evidence",
                    node.node_id
                ))
            })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                ledger_key: projection.ledger_key.clone(),
                required: store::RequiredSideEffectState::ConfirmationObserved,
            });
    }

    Ok(preconditions)
}

fn next_runnable_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
) -> Result<Option<RunnableNode<'a>>> {
    if view
        .projections
        .side_effects()
        .any(|(_, projection)| matches!(projection.phase, store::SideEffectPhase::Ambiguous { .. }))
    {
        return Ok(None);
    }
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        let Some(attempt) = attempt_plan(runtime_spec, node, view)? else {
            continue;
        };
        if node_inputs_ready(runtime_spec, node, view)? {
            return Ok(Some(RunnableNode { node, attempt }));
        }
    }
    Ok(None)
}

fn attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, view)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, view)
    }
}

fn non_side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "node {} attempt {} completed output cell {} without terminal cell authority",
                    node.node_id, attempt_id, output_cell_id
                )));
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        validate_side_effect_terminal_evidence(&view.projections, node, &attempt_id)?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if let Some(projection) =
                    side_effect_projection_for_attempt(&view.projections, node, attempt_id)?
                {
                    match projection.phase {
                        store::SideEffectPhase::Ambiguous { .. } => return Ok(None),
                        store::SideEffectPhase::Failed { .. } => {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "side-effect ledger {} failed while attempt {} for node {} remained started",
                                projection.ledger_key, attempt_id, node.node_id
                            )));
                        }
                        _ => {}
                    }
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                if view.projections.cell_terminal(output_cell_id).is_none() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "side-effect node {} attempt {} completed without terminal output cell {}",
                        node.node_id, attempt_id, output_cell_id
                    )));
                }
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn validate_terminal_cell_has_completed_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    terminal: &store::CellTerminalProjection,
) -> Result<AttemptId> {
    let certified = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node_id, terminal_attempt_id, schema_id, semantic_type_id) = match terminal {
        store::CellTerminalProjection::Produced {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        }
        | store::CellTerminalProjection::Skipped {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        } => (node_id, attempt_id, schema_id, semantic_type_id),
    };
    if terminal_node_id != &node.node_id
        || certified.producer != spec::CellProducer::Node(node.node_id.clone())
        || schema_id != &certified.schema_id
        || semantic_type_id != &certified.semantic_type_id
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} is not certified terminal evidence for node {}",
            node.output_cell, node.node_id
        )));
    }
    match projections.attempt(&node.node_id, terminal_attempt_id) {
        Some(store::AttemptProjection {
            status: store::AttemptStatus::Completed { output_cell_id },
            ..
        }) if output_cell_id == &node.output_cell => Ok(terminal_attempt_id.clone()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} for node {} lacks matching completed attempt {}",
            node.output_cell, node.node_id, terminal_attempt_id
        ))),
    }
}

fn side_effect_projection_for_attempt<'a>(
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == node.node_id
            && projection.intent.attempt_id == *attempt_id
            && found.replace(projection).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledger projections",
                node.node_id, attempt_id
            )));
        }
    }
    Ok(found)
}

fn validate_side_effect_terminal_evidence(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    if matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )))
    }
}

fn recorded_facts_for_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<RecordedFacts> {
    let mut facts = BTreeMap::new();
    for ((fact_node_id, fact_attempt_id, fact_key), projection) in projections.facts() {
        if fact_node_id != node_id || fact_attempt_id != attempt_id {
            continue;
        }
        if projection.node_id != *node_id
            || projection.attempt_id != *attempt_id
            || projection.fact_key != *fact_key
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact projection {} for node {} attempt {} is internally inconsistent",
                fact_key, node_id, attempt_id
            )));
        }
        facts.insert(
            fact_key.clone(),
            RecordedFact {
                fact_key: fact_key.clone(),
                request_schema_id: projection.request_schema_id.clone(),
                request_hash: projection.request_hash.clone(),
                response_schema_id: projection.response_schema_id.clone(),
                response_hash: projection.response_hash.clone(),
                artifact_id: projection.artifact_id.clone(),
                capability_kind: projection.capability_kind.clone(),
                capability_version: projection.capability_version.clone(),
                adapter_kind: projection.adapter_kind.clone(),
                adapter_version: projection.adapter_version.clone(),
            },
        );
    }
    Ok(RecordedFacts { facts })
}

fn node_inputs_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    let input_cells = runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    for cell_id in input_cells {
        let cell = runtime_spec.cell(&cell_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} input cell {} is missing",
                node.node_id, cell_id
            ))
        })?;
        match &cell.producer {
            spec::CellProducer::Seed(_) => {
                if !view.seed_cells.contains_key(&cell_id) {
                    return Ok(false);
                }
            }
            spec::CellProducer::Node(_) => {
                if view.projections.cell_terminal(&cell_id).is_none() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn node_cell_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> Result<Vec<store::CellStatePrecondition>> {
    let mut preconditions = Vec::new();
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        let cell = runtime_spec
            .cell(&cell_id)
            .expect("validated input binding cell exists");
        if matches!(cell.producer, spec::CellProducer::Node(_)) {
            preconditions.push(store::CellStatePrecondition {
                cell_id,
                required: store::RequiredCellState::Terminal,
            });
        }
    }
    Ok(preconditions)
}

fn materialize_inputs(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputs> {
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, &node.input_bindings.root, view)?,
    })
}

fn materialize_input_node(
    runtime_spec: &CertifiedRuntimeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputNode> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, cell, view)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, &field.node, view)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, element, view))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    cell: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedCell> {
    let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {} is not certified by the spec",
            cell.cell_id
        ))
    })?;
    if certified.schema_id != cell.schema_id
        || certified.semantic_type_id != cell.semantic_type_id
        || certified.value_lineage != cell.value_lineage
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} metadata does not match certified cell",
            cell.cell_id
        )));
    }
    let terminal = match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = view.seed_cells.get(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunStarted evidence",
                    cell.cell_id
                ))
            })?;
            MaterializedCellTerminal::Seed {
                seed_id: seed_id.clone(),
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                content_digest: seed.digest.clone(),
            }
        }
        spec::CellProducer::Node(_) => {
            let projection = view
                .projections
                .cell_terminal(&cell.cell_id)
                .ok_or_else(|| {
                    RuntimeError::InputMaterialization(format!(
                        "input cell {} is not terminal",
                        cell.cell_id
                    ))
                })?;
            match projection {
                store::CellTerminalProjection::Produced {
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    ..
                } => {
                    if schema_id != &cell.schema_id || semantic_type_id != &cell.semantic_type_id {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Produced {
                        artifact_id: artifact_id.clone(),
                        content_digest: content_digest.clone(),
                    }
                }
                store::CellTerminalProjection::Skipped {
                    schema_id,
                    semantic_type_id,
                    skip_reason,
                    ..
                } => {
                    if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "input cell {} requires produced terminal but was skipped",
                            cell.cell_id
                        )));
                    }
                    if schema_id != &cell.schema_id || semantic_type_id != &cell.semantic_type_id {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Skipped {
                        skip_reason: skip_reason.clone(),
                    }
                }
            }
        }
    };
    Ok(MaterializedCell {
        cell_id: cell.cell_id.clone(),
        schema_id: cell.schema_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        value_lineage: cell.value_lineage.clone(),
        terminal,
    })
}

fn validate_seed_cells(
    runtime_spec: &CertifiedRuntimeSpec,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>> {
    let mut by_seed = BTreeMap::<_, _>::new();
    let mut by_cell = BTreeMap::<_, _>::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &runtime_spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing RunStarted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} metadata does not match certified seed",
                declared.seed_id
            )));
        }
        if let Some(required_digest) = &declared.required_digest {
            if &seed.digest != required_digest {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "RunStarted seed {} digest does not match certified required digest",
                    declared.seed_id
                )));
            }
        }
        if seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} artifact evidence does not match certified seed",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != runtime_spec.spec().seeds.len() {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted contains seed evidence not certified by the spec".to_owned(),
        ));
    }
    Ok(by_cell)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoricalSideEffectPhase {
    IntentPersisted,
    Claimed,
    InvocationPrepared,
    InvocationStarted,
    NotSubmittedProven,
    SubmissionObserved,
    SubmissionUnknown,
    ReceiptObserved,
    ConfirmationObserved,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoricalSideEffectLedger {
    node_id: NodeId,
    attempt_id: AttemptId,
    phase: HistoricalSideEffectPhase,
}

fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let mut available_cells = BTreeSet::<CellId>::new();
    let mut active_attempts = BTreeSet::<(NodeId, AttemptId)>::new();
    let mut side_effect_ledgers =
        BTreeMap::<events::SideEffectLedgerKey, HistoricalSideEffectLedger>::new();
    let mut seen_run_started = false;
    for event in stream {
        if !seen_run_started
            && !matches!(event.payload(), events::KernelEventPayload::RunStarted(_))
        {
            return Err(RuntimeError::InvalidRunStream(
                "run stream events appeared before RunStarted".to_owned(),
            ));
        }
        match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => {
                if seen_run_started {
                    return Err(RuntimeError::InvalidRunStream(
                        "run stream contains multiple RunStarted events".to_owned(),
                    ));
                }
                seen_run_started = true;
                for cell_id in validate_seed_cells(runtime_spec, &payload.seed_cells)?.into_keys() {
                    available_cells.insert(cell_id);
                }
            }
            events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_) => {}
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt started for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.state_kind != node.state_kind
                    || payload.state_version != node.state_version
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} carries state identity outside the certified spec",
                        payload.attempt_id, payload.node_id
                    )));
                }
                validate_attempt_start_boundary(runtime_spec, node, payload, &available_cells)?;
                if !active_attempts.insert((payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} was started more than once",
                        payload.attempt_id, payload.node_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} completed uncertified output cell {}",
                        payload.attempt_id, payload.node_id, payload.output_cell_id
                    )));
                }
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if runtime_spec
                    .node(&payload.node_id)
                    .expect("checked above")
                    .side_effect
                    .is_some()
                {
                    validate_historical_side_effect_failure(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt failure for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_historical_produced_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "fact recorded for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let caps = CertifiedRuntimeCapabilities::new(
                    node.node_id.clone(),
                    node.capability_bindings.clone(),
                );
                require_capability(
                    &caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_projected_attempt(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    "fact",
                )?;
                if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} was recorded outside an active started attempt",
                        payload.fact_key, payload.node_id, payload.attempt_id
                    )));
                }
                let fact = projections
                    .fact(&payload.node_id, &payload.attempt_id, &payload.fact_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "fact {} for node {} attempt {} is not projected",
                            payload.fact_key, payload.node_id, payload.attempt_id
                        ))
                    })?;
                if fact.event_id != *event.event_id()
                    || fact.request_schema_id != payload.request_schema_id
                    || fact.request_hash != payload.request_hash
                    || fact.response_schema_id != payload.response_schema_id
                    || fact.response_hash != payload.response_hash
                    || fact.artifact_id != payload.artifact_id
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} projection does not match authoritative event",
                        payload.fact_key
                    )));
                }
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if let Some(node_id) = &payload.node_id {
                    runtime_spec.node(node_id).ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "artifact referenced for uncertified node {node_id}"
                        ))
                    })?;
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output(runtime_spec, node, payload)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_produced(
                    runtime_spec,
                    projections,
                    event,
                    node,
                    payload,
                )?;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output failure by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_failed(projections, event, payload)?;
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_historical_side_effect_payload(
                    runtime_spec,
                    &active_attempts,
                    &mut side_effect_ledgers,
                    event.payload(),
                )?;
            }
        }
    }
    validate_atomic_terminal_pairs(runtime_spec, stream)?;
    validate_atomic_side_effect_failure_pairs(runtime_spec, stream)?;
    validate_recovery_frontier(runtime_spec, projections)?;
    Ok(())
}

fn validate_historical_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let (node_id, attempt_id, ledger_key, phase) = side_effect_payload_ref(payload)
        .ok_or_else(|| RuntimeError::InvalidRunStream("expected side-effect payload".to_owned()))?;
    let node = runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("side-effect event for uncertified node {node_id}"))
    })?;
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "non-side-effect node {} emitted side-effect ledger event",
            node.node_id
        )));
    }
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect ledger {} for node {} attempt {} was recorded outside an active started attempt",
            ledger_key, node_id, attempt_id
        )));
    }

    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect intent for node {} carries uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            let caps = CertifiedRuntimeCapabilities::new(
                node.node_id.clone(),
                node.capability_bindings.clone(),
            );
            require_capability(
                &caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            if ledgers
                .insert(
                    ledger_key.clone(),
                    HistoricalSideEffectLedger {
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        phase,
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} persisted intent more than once",
                    ledger_key
                )));
            }
        }
        _ => {
            let ledger = ledgers.get_mut(ledger_key).ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} advanced before intent was persisted",
                    ledger_key
                ))
            })?;
            if ledger.node_id != *node_id || ledger.attempt_id != *attempt_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed node or attempt authority",
                    ledger_key
                )));
            }
            ledger.phase = phase;
        }
    }

    Ok(())
}

fn side_effect_payload_ref(
    payload: &events::KernelEventPayload,
) -> Option<(
    &NodeId,
    &AttemptId,
    &events::SideEffectLedgerKey,
    HistoricalSideEffectPhase,
)> {
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::IntentPersisted,
        )),
        events::KernelEventPayload::SideEffectClaimed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationPrepared,
        )),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationStarted,
        )),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::NotSubmittedProven,
        )),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionObserved,
        )),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionUnknown,
        )),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ReceiptObserved,
        )),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ConfirmationObserved,
        )),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Ambiguous,
        )),
        events::KernelEventPayload::SideEffectFailed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Failed,
        )),
        _ => None,
    }
}

fn validate_historical_side_effect_confirmation(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node_id, attempt_id
        )))
    }
}

fn validate_historical_side_effect_failure(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::Failed {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} failed without side-effect failure evidence",
            node_id, attempt_id
        )))
    }
}

fn historical_side_effect_ledger_for_attempt<'a>(
    ledgers: &'a BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<&'a HistoricalSideEffectLedger> {
    let mut found = None;
    for ledger in ledgers.values() {
        if ledger.node_id == *node_id
            && ledger.attempt_id == *attempt_id
            && found.replace(ledger).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledgers in history",
                node_id, attempt_id
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} lacks ledger evidence",
            node_id, attempt_id
        ))
    })
}

fn validate_attempt_start_boundary(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::StateAttemptStarted,
    available_cells: &BTreeSet<CellId>,
) -> Result<()> {
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        if !available_cells.contains(&cell_id) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} started before certified input cell {} was terminal",
                payload.attempt_id, node.node_id, cell_id
            )));
        }
    }
    Ok(())
}

fn validate_atomic_terminal_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                completions.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell skipped by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (seq, node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            output_cell_id.clone(),
        )) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} completed without terminal cell {} in the same commit",
                attempt_id, node_id, output_cell_id
            )));
        }
    }
    for (seq, node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(*seq, node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "terminal cell {} for node {} lacks StateAttemptCompleted in the same commit",
                cell_id, node_id
            )));
        }
    }
    Ok(())
}

fn validate_atomic_side_effect_failure_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::SideEffectFailed(payload) => {
                side_effect_failures.insert(
                    (
                        event.seq(),
                        payload.node_id.clone(),
                        payload.attempt_id.clone(),
                    ),
                    payload.retryable,
                );
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node.side_effect.is_some() {
                    attempt_failures.insert(
                        (
                            event.seq(),
                            payload.node_id.clone(),
                            payload.attempt_id.clone(),
                        ),
                        payload.retryable,
                    );
                }
            }
            _ => {}
        }
    }
    for (failure, side_effect_retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
            Some(_) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} disagrees with StateAttemptFailed retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} lacks StateAttemptFailed in the same commit",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect attempt failure for node {} attempt {} lacks SideEffectFailed in the same commit",
                failure.1, failure.2
            )));
        }
    }
    Ok(())
}

fn validate_recovery_frontier(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if let Some(terminal) = projections.cell_terminal(&node.output_cell) {
            let attempt_id = validate_terminal_cell_has_completed_attempt(
                runtime_spec,
                projections,
                node,
                terminal,
            )?;
            if node.side_effect.is_some() {
                validate_side_effect_terminal_evidence(projections, node, &attempt_id)?;
            }
        }
        let mut started = None;
        for ((attempt_node_id, attempt_id), projection) in projections.attempts() {
            if attempt_node_id != &node.node_id {
                continue;
            }
            match &projection.status {
                store::AttemptStatus::Started { .. } => {
                    if started.replace(attempt_id.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has multiple started attempts during recovery",
                            node.node_id
                        )));
                    }
                }
                store::AttemptStatus::Completed { output_cell_id }
                    if projections.cell_terminal(output_cell_id).is_none() =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} attempt {} completed without terminal cell projection",
                        node.node_id, attempt_id
                    )));
                }
                store::AttemptStatus::Completed { .. } | store::AttemptStatus::Failed { .. } => {}
            }
        }
    }
    Ok(())
}

fn require_projected_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_kind: &'static str,
) -> Result<store::AttemptStatus> {
    let attempt = projections.attempt(node_id, attempt_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "{event_kind} event references missing attempt {attempt_id} for node {node_id}"
        ))
    })?;
    Ok(attempt.status.clone())
}

fn validate_historical_produced_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellProduced,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("produced uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was produced by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_skipped_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellSkipped,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("skipped uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was skipped by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && skip_reason == &payload.skip_reason =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_public_output_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output",
    )? {
        store::AttemptStatus::Completed { output_cell_id }
            if output_cell_id == node.output_cell => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} is not backed by a completed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    if projections.cell_terminal(&node.output_cell).is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} has no terminal render receipt cell",
            payload.node_id
        )));
    }
    for cell in &payload.cells {
        let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "public output references uncertified cell {}",
                cell.cell_id
            ))
        })?;
        match projections.cell_terminal(&cell.cell_id) {
            Some(store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && certified.producer == cell.producer
                && certified.scope_id == cell.scope_id
                && certified.value_lineage == cell.value_lineage => {}
            _ => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "public output source cell {} is not backed by matching terminal evidence",
                    cell.cell_id
                )));
            }
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        }) if event_id == event.event_id()
            && rendered_digest == &payload.rendered_digest
            && rendered_artifact_id == &payload.rendered_artifact_id =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output projection for schema {} does not match authoritative event",
            payload.public_schema_id
        ))),
    }
}

fn validate_historical_public_output_failed(
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output failure",
    )? {
        store::AttemptStatus::Failed { .. } => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output failure for node {} attempt {} is not backed by a failed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::RenderFailed { event_id, .. })
            if event_id == event.event_id() =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output failure projection for schema {} does not match authoritative event",
            payload.public_schema_id
        ))),
    }
}

fn validate_spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != runtime_spec.spec().media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedExecutionSpec
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed execution spec artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

fn validate_config_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: Vec<store::ArtifactEvidenceRef>,
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut by_key = BTreeMap::new();
    for artifact in evidence {
        let Some(schema_id) = artifact.schema_id.clone() else {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence must carry schema_id".to_owned(),
            ));
        };
        if artifact.artifact_role != events::ArtifactRole::TypedConfig
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence has invalid role or producer metadata".to_owned(),
            ));
        }
        let key = format!("{}:{}", schema_id, artifact.digest);
        if by_key.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate typed config artifact evidence".to_owned(),
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config in &runtime_spec.spec().config_refs {
        let key = config_ref_key(config);
        let artifact = by_key.remove(&key).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing typed config artifact evidence for schema {} digest {}",
                config.schema_id, config.digest
            ))
        })?;
        if artifact.artifact_id != config.artifact_id
            || artifact.digest != config.digest
            || artifact.byte_len != config.byte_len
            || artifact.media_type != config.media_type
            || artifact.schema_id.as_ref() != Some(&config.schema_id)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "typed config artifact evidence for schema {} digest {} does not match certified config ref",
                config.schema_id, config.digest
            )));
        }
        validated.push(artifact);
    }
    if !by_key.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "typed config artifact evidence contains entries not certified by the spec".to_owned(),
        ));
    }
    Ok(validated)
}

fn validate_runner_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    recorded_facts: &RecordedFacts,
    projections: &store::ProjectionSnapshot,
    output: &ErasedRunnerOutput,
) -> Result<()> {
    if output.payloads.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let mut completed = false;
    let mut failed = false;
    let mut terminal_cell = false;
    let mut public_output_produced = false;
    let mut public_output_failed = false;
    let mut side_effect_payload = false;
    let mut side_effect_failed = false;
    let mut attempt_failure_retryable = None;
    let mut side_effect_failure_retryable = None;
    for payload in &output.payloads {
        if payload_spec_hash(payload) != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} returned payload with mismatched spec hash",
                node.node_id
            )));
        }
        match payload {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} completed output cell {} instead of certified {}",
                        node.node_id, payload.output_cell_id, node.output_cell
                    )));
                }
                completed = true;
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if failed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
                attempt_failure_retryable = Some(payload.retryable);
                failed = true;
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced cell metadata outside certified spec",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped a cell outside certified skip policy",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if !recorded_facts.is_empty() {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} attempted to record fact {} after committed facts existed for the same attempt",
                        node.node_id, payload.fact_key
                    )));
                }
                require_capability(
                    caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if payload.node_id.as_ref() != Some(&node.node_id)
                    || payload.attempt_id.as_ref() != Some(attempt_id)
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} referenced artifact outside its attempt",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output(runtime_spec, node, payload)?;
                public_output_produced = true;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )?;
                public_output_failed = true;
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_runner_side_effect_payload(node, attempt_id, caps, projections, payload)?;
                side_effect_payload = true;
                if let events::KernelEventPayload::SideEffectFailed(payload) = payload {
                    side_effect_failed = true;
                    side_effect_failure_retryable = Some(payload.retryable);
                }
            }
        }
    }
    if node.side_effect.is_some() {
        validate_side_effect_resume_output(projections, node, attempt_id, &output.payloads)?;
        if failed {
            if !side_effect_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned StateAttemptFailed without SideEffectFailed",
                    node.node_id
                )));
            }
            if side_effect_failure_retryable != attempt_failure_retryable {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned inconsistent failure retryability",
                    node.node_id
                )));
            }
            if completed || terminal_cell || public_output_produced {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} mixed side-effect failure with successful terminal evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if side_effect_failed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} returned SideEffectFailed without StateAttemptFailed",
                node.node_id
            )));
        }
        if side_effect_payload {
            if completed || terminal_cell || public_output_produced || public_output_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} mixed ledger phase events with terminal output evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if !completed || !terminal_cell {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} output commit must pair StateAttemptCompleted with terminal cell evidence",
                node.node_id
            )));
        }
        validate_side_effect_terminal_evidence(projections, node, attempt_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if public_output_produced && !completed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} projected public output without completing its certified output cell",
                node.node_id
            )));
        }
        return Ok(());
    }
    if failed && (completed || terminal_cell || public_output_produced) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} mixed failure with successful terminal evidence",
            node.node_id
        )));
    }
    if public_output_failed && !failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned public-output failure without StateAttemptFailed",
            node.node_id
        )));
    }
    if !failed && (!completed || !terminal_cell) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_produced && !completed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} projected public output without completing its certified output cell",
            node.node_id
        )));
    }
    Ok(())
}

fn validate_runner_side_effect_payload(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    }
    let (payload_node_id, payload_attempt_id, _, _) =
        side_effect_payload_ref(payload).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} persisted intent for uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            require_capability(
                caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
        }
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            if payload.new_claim_owner == payload.previous_claim_owner {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover reused the previous claim owner",
                    node.node_id
                )));
            }
            if payload.claim_generation <= payload.previous_claim_generation {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover did not increase claim generation",
                    node.node_id
                )));
            }
            if let Some(projection) =
                side_effect_projection_for_attempt(projections, node, attempt_id)?
            {
                let claim = projection.claim.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover requires an active claim",
                        node.node_id
                    ))
                })?;
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover reused the previous fencing token",
                        node.node_id
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_side_effect_resume_output(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Ok(());
    };
    let has_takeover = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
        )
    });
    let has_claim = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::SideEffectClaimed(_)));
    let has_invocation_started = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )
    });
    let has_submission_recovery = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectNotSubmittedProven(_)
                | events::KernelEventPayload::SideEffectSubmissionObserved(_)
                | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                | events::KernelEventPayload::SideEffectAmbiguous(_)
        )
    });

    match projection.phase {
        store::SideEffectPhase::Claimed { .. }
        | store::SideEffectPhase::InvocationPrepared { .. } => {
            if !has_takeover {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed pre-invocation ledger {} without claim takeover",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::NotSubmittedProven { .. } => {
            if !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationStarted { .. }
        | store::SideEffectPhase::SubmissionUnknown { .. } => {
            if !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::Ambiguous { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run ambiguous ledger {}",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::Failed { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run failed ledger {} on the same attempt",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::IntentPersisted { .. }
        | store::SideEffectPhase::SubmissionObserved { .. }
        | store::SideEffectPhase::ReceiptObserved { .. }
        | store::SideEffectPhase::ConfirmationObserved { .. } => {}
    }

    if has_invocation_started
        && matches!(projection.phase, store::SideEffectPhase::Claimed { .. })
        && !has_takeover
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} started invocation from stale claim without takeover",
            node.node_id
        )));
    }

    Ok(())
}

fn validate_public_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    validate_public_output_render_node(
        runtime_spec,
        node,
        payload.public_schema_id.clone(),
        &payload.renderer_descriptor_id,
    )?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if payload.output_spec_digest != render.output_spec_digest
        || payload.receipt_cell_id != node.output_cell
        || payload.cells.len() != render.required_cells.len()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output payload for node {} does not match certified render contract",
            node.node_id
        )));
    }
    for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
        if actual.public_field_path != expected.public_field_path
            || actual.cell_id != expected.cell_id
            || actual.producer != expected.producer
            || actual.scope_id != expected.scope_id
            || actual.semantic_type_id != expected.semantic_type_id
            || actual.schema_id != expected.schema_id
            || actual.value_lineage != expected.value_lineage
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output cell evidence for node {} does not match certified output",
                node.node_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_render_node(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    public_schema_id: SchemaId,
    renderer_descriptor_id: &DescriptorId,
) -> Result<()> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if render.public_schema_id != public_schema_id
        || runtime_spec.spec().public_outputs.public_schema_id != public_schema_id
        || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output render metadata for node {} does not match certified spec",
            node.node_id
        )));
    }
    Ok(())
}

fn require_attempt(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    actual_node_id: &NodeId,
    actual_attempt_id: &AttemptId,
) -> Result<()> {
    if actual_node_id == &node.node_id && actual_attempt_id == attempt_id {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "payload for node {} attempt {} was bound to node {} attempt {}",
            node.node_id, attempt_id, actual_node_id, actual_attempt_id
        )))
    }
}

fn require_capability(
    caps: &CertifiedRuntimeCapabilities,
    kind: &CapabilityKind,
    version: &CapabilityVersion,
    node_id: &NodeId,
) -> Result<()> {
    if caps.contains(kind, version) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {node_id} used uncertified capability {kind}:{version}"
        )))
    }
}

fn require_adapter(
    node: &spec::NodeSpec,
    kind: &AdapterKind,
    version: &AdapterVersion,
) -> Result<()> {
    if node
        .adapter_bindings
        .iter()
        .any(|adapter| adapter.adapter_kind == *kind && adapter.adapter_version == *version)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} used uncertified adapter {}:{}",
            node.node_id, kind, version
        )))
    }
}

fn next_attempt_no(projections: &store::ProjectionSnapshot, node_id: &NodeId) -> Result<u32> {
    let count = projections
        .attempts()
        .filter(|((attempt_node_id, _), _)| attempt_node_id == node_id)
        .count();
    u32::try_from(count + 1).map_err(|_| {
        RuntimeError::InvalidRunStream(format!("attempt count overflow for {node_id}"))
    })
}

fn attempt_id(
    run_id: &RunId,
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_no: u32,
) -> Result<AttemptId> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_no": attempt_no,
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
        "spec_hash": spec_hash.as_str(),
    }))?;
    Ok(AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *canonical.content_digest().digest(),
    ))
}

fn payload_spec_hash(payload: &events::KernelEventPayload) -> SpecHash {
    match payload {
        events::KernelEventPayload::RunStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::FactRecorded(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellSkipped(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RunCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash.clone()
        }
    }
}

fn store_seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        digest: seed.seed_artifact.content_digest.clone(),
        byte_len: seed.seed_artifact.byte_len,
        media_type: seed.seed_artifact.media_type.clone(),
        schema_id: Some(seed.seed_artifact.schema_id.clone()),
        semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: seed.seed_artifact.role,
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_capabilities::CapabilityRole;
    use mfm_ids::{
        DigestBytes, EffectKind, EffectVersion, ScopeId, SeedId, SemanticTypeId, StateKind,
        StateVersion,
    };
    use mfm_store::v1::{TypedProjectionRead, TypedRunEventStore};

    const D0: DigestBytes = DigestBytes::from_array([0x10; 32]);
    const D1: DigestBytes = DigestBytes::from_array([0x11; 32]);
    const D2: DigestBytes = DigestBytes::from_array([0x12; 32]);
    const D3: DigestBytes = DigestBytes::from_array([0x13; 32]);
    const D4: DigestBytes = DigestBytes::from_array([0x14; 32]);
    const D5: DigestBytes = DigestBytes::from_array([0x15; 32]);
    const D6: DigestBytes = DigestBytes::from_array([0x16; 32]);
    const D7: DigestBytes = DigestBytes::from_array([0x17; 32]);
    const D8: DigestBytes = DigestBytes::from_array([0x18; 32]);
    const D9: DigestBytes = DigestBytes::from_array([0x19; 32]);
    const DA: DigestBytes = DigestBytes::from_array([0x1a; 32]);
    const DB: DigestBytes = DigestBytes::from_array([0x1b; 32]);
    const DC: DigestBytes = DigestBytes::from_array([0x1c; 32]);
    const DD: DigestBytes = DigestBytes::from_array([0x1d; 32]);
    const DE: DigestBytes = DigestBytes::from_array([0x1e; 32]);
    const DF: DigestBytes = DigestBytes::from_array([0x1f; 32]);

    #[derive(Clone)]
    struct Fixture {
        runtime_spec: CertifiedRuntimeSpec,
        run_id: RunId,
        seed_ref: events::SeedCellRef,
        descriptor_a: DescriptorId,
        descriptor_b: DescriptorId,
        cell_a: CellId,
        cell_b: CellId,
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    struct RecordingRunner {
        expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for RecordingRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                for (kind, version) in &self.expected_caps {
                    assert!(ctx.caps.contains(kind, version));
                }
                let output_cell = ctx.node.output_cell.clone();
                let cell = match ctx.inputs.root.clone() {
                    MaterializedInputNode::Cell(cell) => cell,
                    MaterializedInputNode::Unit
                    | MaterializedInputNode::Tuple(_)
                    | MaterializedInputNode::Struct(_)
                    | MaterializedInputNode::Vec(_)
                    | MaterializedInputNode::NonEmptyVec(_) => {
                        panic!("expected cell input")
                    }
                };
                assert!(matches!(
                    cell.terminal,
                    MaterializedCellTerminal::Seed { .. }
                        | MaterializedCellTerminal::Produced { .. }
                ));
                let certified_cell = ctx.projections.cell_terminal(&output_cell).is_none();
                assert!(certified_cell);
                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor.output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor.output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                Ok(ErasedRunnerOutput {
                    required_artifacts: vec![artifact],
                    payloads: vec![
                        events::KernelEventPayload::CellProduced(events::CellProduced {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            cell_id: ctx.node.output_cell.clone(),
                            scope_id: ctx.node.scope_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            semantic_type_id: ctx.descriptor.output_semantic_type_id.clone(),
                            schema_id: ctx.descriptor.output_schema_id.clone(),
                            value_lineage: ctx.output_cell.value_lineage.clone(),
                            artifact_id: self.output_artifact.clone(),
                            content_digest: self.output_digest.clone(),
                            producer_state_kind: Some(ctx.node.state_kind.clone()),
                            producer_state_version: Some(ctx.node.state_version.clone()),
                        }),
                        events::KernelEventPayload::StateAttemptCompleted(
                            events::StateAttemptCompleted {
                                spec_hash: ctx.spec_hash.clone(),
                                node_id: ctx.node.node_id.clone(),
                                attempt_id: ctx.attempt_id.clone(),
                                output_cell_id: ctx.node.output_cell.clone(),
                            },
                        ),
                    ],
                })
            })
        }
    }

    #[tokio::test]
    async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive a"),
            SchedulerStatus::Advanced
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_some());
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_none());
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive b"),
            SchedulerStatus::Advanced
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_some());
    }

    #[test]
    fn certified_runtime_spec_rejects_hash_mismatch() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
        assert!(matches!(
            CertifiedRuntimeSpec::new(envelope),
            Err(RuntimeError::SpecHash(_))
        ));
    }

    #[tokio::test]
    async fn scheduler_rejects_uncertified_capability_use() {
        struct BadFactRunner {
            cap_kind: CapabilityKind,
            cap_version: CapabilityVersion,
        }

        impl ErasedNodeRunner for BadFactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput::new(vec![
                        events::KernelEventPayload::FactRecorded(events::FactRecorded {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: AdapterKind::new(
                                "mfm.test",
                                "adapter",
                                DigestAlgorithm::Sha256JcsV1,
                                D1,
                            )
                            .expect("adapter"),
                            adapter_version: AdapterVersion::new("mfm.adapter.v1")
                                .expect("adapter version"),
                            request_schema_id: ctx.node.config_ref.schema_id.clone(),
                            request_hash: content(0xc1),
                            response_schema_id: ctx.node.config_ref.schema_id.clone(),
                            response_hash: content(0xc2),
                            fact_key: events::FactKey::new("bad-fact").expect("fact key"),
                            artifact_id: artifact(0xc3),
                        }),
                    ]))
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                BadFactRunner {
                    cap_kind: fixture.cap_kind.clone(),
                    cap_version: fixture.cap_version.clone(),
                },
            ))
            .expect("binding");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    #[test]
    fn materialization_rejects_seed_digest_not_certified() {
        let fixture = fixture();
        let mut seed = fixture.seed_ref.clone();
        seed.digest = content(0xee);
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        assert!(matches!(
            scheduler.start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![seed]),
            ),
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[test]
    fn run_start_rejects_missing_config_artifact_evidence() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
        evidence.config_artifacts.clear();
        assert!(matches!(
            scheduler.start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                evidence,
            ),
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let forged_node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell != fixture.cell_a)
            .expect("second node")
            .clone();
        let certified_cell = fixture
            .runtime_spec
            .cell(&fixture.cell_a)
            .expect("cell a")
            .clone();
        let forged_attempt = AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xfa; 32]),
        );
        let artifact_id = artifact(0xfa);
        let artifact_digest = content(0xfb);
        store
            .record_artifact_evidence(store::ArtifactEvidenceRef {
                artifact_id: artifact_id.clone(),
                digest: artifact_digest.clone(),
                byte_len: 10,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(certified_cell.schema_id.clone()),
                semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
                producer_node_id: Some(forged_node.node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::StateOutput,
            })
            .expect("record forged artifact");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
                payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                    events::StateAttemptStarted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: forged_node.node_id.clone(),
                        attempt_id: forged_attempt.clone(),
                        attempt_no: 1,
                        state_kind: forged_node.state_kind.clone(),
                        state_version: forged_node.state_version.clone(),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged attempt start");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: forged_node.node_id.clone(),
                        cell_id: fixture.cell_a.clone(),
                        scope_id: certified_cell.scope_id.clone(),
                        attempt_id: forged_attempt.clone(),
                        semantic_type_id: certified_cell.semantic_type_id.clone(),
                        schema_id: certified_cell.schema_id.clone(),
                        value_lineage: certified_cell.value_lineage.clone(),
                        artifact_id,
                        content_digest: artifact_digest,
                        producer_state_kind: Some(forged_node.state_kind.clone()),
                        producer_state_version: Some(forged_node.state_version.clone()),
                    }),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: forged_node.node_id.clone(),
                            attempt_id: forged_attempt,
                            output_cell_id: fixture.cell_a.clone(),
                        },
                    ),
                ],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged terminal");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[test]
    fn store_rejects_fact_without_started_attempt() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell == fixture.cell_b)
            .expect("read node")
            .clone();
        let fact_artifact = artifact(0xd1);
        let fact_digest = content(0xd2);
        let fact_schema = node.config_ref.schema_id.clone();
        store
            .record_artifact_evidence(store::ArtifactEvidenceRef {
                artifact_id: fact_artifact.clone(),
                digest: fact_digest.clone(),
                byte_len: 10,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(fact_schema.clone()),
                semantic_type_id: None,
                producer_node_id: Some(node.node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::FactResponse,
            })
            .expect("record fact artifact");
        assert!(store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
                payloads: vec![events::KernelEventPayload::FactRecorded(
                    events::FactRecorded {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: AttemptId::from_digest(
                            DigestAlgorithm::Sha256JcsV1,
                            DigestBytes::from_array([0xd3; 32]),
                        ),
                        capability_kind: fixture.cap_kind.clone(),
                        capability_version: fixture.cap_version.clone(),
                        adapter_kind: fixture.adapter_kind.clone(),
                        adapter_version: fixture.adapter_version.clone(),
                        request_schema_id: fact_schema.clone(),
                        request_hash: content(0xd4),
                        response_schema_id: fact_schema,
                        response_hash: fact_digest,
                        fact_key: events::FactKey::new("forged-fact").expect("fact key"),
                        artifact_id: fact_artifact,
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .is_err());
    }

    #[tokio::test]
    async fn replay_rejects_public_output_without_render_attempt() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let non_render_node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell == fixture.cell_a)
            .expect("non-render node")
            .clone();
        let public_cell = fixture
            .runtime_spec
            .spec()
            .public_outputs
            .outputs
            .first()
            .expect("public cell")
            .clone();
        let source_artifact = artifact(0xe1);
        let source_digest = content(0xe2);
        let producer_node_id = match &public_cell.producer {
            spec::CellProducer::Node(node_id) => Some(node_id.clone()),
            spec::CellProducer::Seed(_) => None,
        };
        store
            .record_artifact_evidence(store::ArtifactEvidenceRef {
                artifact_id: source_artifact.clone(),
                digest: source_digest.clone(),
                byte_len: 10,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(public_cell.schema_id.clone()),
                semantic_type_id: Some(public_cell.semantic_type_id.clone()),
                producer_node_id,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::StateOutput,
            })
            .expect("record source artifact");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-public-output").expect("commit key"),
                payloads: vec![events::KernelEventPayload::PublicOutputProduced(
                    events::PublicOutputProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: non_render_node.node_id.clone(),
                        attempt_id: AttemptId::from_digest(
                            DigestAlgorithm::Sha256JcsV1,
                            DigestBytes::from_array([0xe3; 32]),
                        ),
                        receipt_cell_id: non_render_node.output_cell.clone(),
                        public_schema_id: fixture
                            .runtime_spec
                            .spec()
                            .public_outputs
                            .public_schema_id
                            .clone(),
                        output_spec_digest: fixture
                            .runtime_spec
                            .spec()
                            .public_outputs
                            .digest()
                            .expect("public digest"),
                        cells: vec![events::NamedTypedCellRef {
                            public_field_path: public_cell.public_field_path.clone(),
                            cell_id: public_cell.cell_id.clone(),
                            producer: public_cell.producer.clone(),
                            scope_id: public_cell.scope_id.clone(),
                            semantic_type_id: public_cell.semantic_type_id.clone(),
                            schema_id: public_cell.schema_id.clone(),
                            value_lineage: public_cell.value_lineage.clone(),
                            content_digest: source_digest,
                            artifact_id: source_artifact,
                        }],
                        rendered_digest: content(0xe4),
                        rendered_artifact_id: None,
                        renderer_descriptor_id: fixture
                            .runtime_spec
                            .spec()
                            .public_outputs
                            .renderer_descriptor
                            .descriptor_id
                            .clone(),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged public output");
        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn recovery_continues_started_pure_attempt_with_same_attempt_id() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);

        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("resume pure"),
            SchedulerStatus::Advanced
        );
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce first cell");
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        corrupt_stream.retain(|event| {
            !matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.output_cell_id == fixture.cell_a
            )
        });
        let projection =
            store::ProjectionSnapshot::rebuild_from_run_stream(&corrupt_stream).expect("rebuild");
        let mut corrupt_store = ReadOnlyCorruptStore {
            stream: corrupt_stream,
            projection,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
        let fixture = fixture();
        let scheduler = SerialTypedScheduler::new(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node_a = node_by_output(&fixture, &fixture.cell_a);
        let node_b = node_by_output(&fixture, &fixture.cell_b);
        append_attempt_start(&mut store, &fixture, node_b, 1);
        let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
        append_terminal(
            &mut store,
            &fixture,
            node_a,
            &attempt_a,
            artifact(0xa1),
            content(0xa2),
        );

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn recovery_reuses_committed_read_facts_for_same_attempt() {
        struct FactReuseRunner {
            fact_key: events::FactKey,
            output_artifact: ArtifactId,
            output_digest: ContentDigest,
        }

        impl ErasedNodeRunner for FactReuseRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let fact = ctx
                        .recorded_facts
                        .get(&self.fact_key)
                        .expect("recorded fact");
                    assert_eq!(fact.fact_key, self.fact_key);
                    assert_eq!(fact.request_schema_id, ctx.node.config_ref.schema_id);
                    assert_eq!(ctx.recorded_facts.iter().count(), 1);
                    let artifact = store::ArtifactEvidenceRef {
                        artifact_id: self.output_artifact.clone(),
                        digest: self.output_digest.clone(),
                        byte_len: 17,
                        media_type: spec::MediaType::new("application/json").expect("media"),
                        schema_id: Some(ctx.descriptor.output_schema_id.clone()),
                        semantic_type_id: Some(ctx.descriptor.output_semantic_type_id.clone()),
                        producer_node_id: Some(ctx.node.node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::StateOutput,
                    };
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![artifact],
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let fact_key = events::FactKey::new("reused-fact").expect("fact key");
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                FactReuseRunner {
                    fact_key: fact_key.clone(),
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce input");
        let node = node_by_output(&fixture, &fixture.cell_b);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        append_fact(
            &mut store,
            &fixture,
            node,
            &attempt_id,
            fact_key.clone(),
            artifact(0xd1),
            content(0xd2),
        );

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume read");
        assert_eq!(fact_recorded_count(&store), 1);
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn recovery_rejects_new_fact_after_same_attempt_fact_exists() {
        struct NewFactRunner {
            cap_kind: CapabilityKind,
            cap_version: CapabilityVersion,
            adapter_kind: AdapterKind,
            adapter_version: AdapterVersion,
        }

        impl ErasedNodeRunner for NewFactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput::new(vec![
                        events::KernelEventPayload::FactRecorded(events::FactRecorded {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                            request_schema_id: ctx.node.config_ref.schema_id.clone(),
                            request_hash: content(0xe1),
                            response_schema_id: ctx.node.config_ref.schema_id.clone(),
                            response_hash: content(0xe2),
                            fact_key: events::FactKey::new("new-fact").expect("fact key"),
                            artifact_id: artifact(0xe3),
                        }),
                    ]))
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                NewFactRunner {
                    cap_kind: fixture.cap_kind.clone(),
                    cap_version: fixture.cap_version.clone(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce input");
        let node = node_by_output(&fixture, &fixture.cell_b);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        append_fact(
            &mut store,
            &fixture,
            node,
            &attempt_id,
            events::FactKey::new("existing-fact").expect("fact key"),
            artifact(0xd1),
            content(0xd2),
        );

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
        assert_eq!(store.projection_snapshot().facts().count(), 1);
    }

    #[tokio::test]
    async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
        let fixture = fixture_with_first_managed_write_state();
        let output_artifact = artifact(0xa1);
        let output_digest = content(0xa2);
        let node = node_by_output(&fixture, &fixture.cell_a);
        let expected_caps = node
            .capability_bindings
            .capabilities
            .iter()
            .map(|capability| (capability.kind.clone(), capability.version.clone()))
            .collect();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "managed-write",
                RecordingRunner {
                    expected_caps,
                    output_artifact: output_artifact.clone(),
                    output_digest: output_digest.clone(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        let descriptor = fixture
            .runtime_spec
            .state_descriptor_for_node(node)
            .expect("descriptor");
        store
            .record_artifact_evidence(state_output_artifact(
                node,
                descriptor,
                output_artifact,
                output_digest,
            ))
            .expect("stage managed artifact");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume managed write");
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = SerialTypedScheduler::new(registered_side_effect_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..6 {
            assert_eq!(
                scheduler
                    .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                    .await
                    .expect("drive side effect phase"),
                SchedulerStatus::Advanced
            );
        }

        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_some());
        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        let projection =
            side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        assert!(matches!(
            projection.phase,
            store::SideEffectPhase::ConfirmationObserved { .. }
        ));
        assert!(store
            .load_run_stream(&fixture.run_id)
            .iter()
            .any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectClaimTakenOver(_)
            )));
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
    }

    #[tokio::test]
    async fn side_effect_not_submitted_resume_claims_next_epoch() {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = SerialTypedScheduler::new(registered_side_effect_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("prepare side effect");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("take over and start invocation");

        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume not-submitted");
        let projection =
            side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        assert!(matches!(
            projection.phase,
            store::SideEffectPhase::InvocationStarted {
                invocation_epoch: 2,
                claim_generation: 3,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn side_effect_ambiguous_phase_blocks_resume() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                AmbiguousSideEffectRunner::new(&fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..3 {
            assert_eq!(
                scheduler
                    .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                    .await
                    .expect("advance to ambiguity"),
                SchedulerStatus::Advanced
            );
        }
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("ambiguous side effect blocks"),
            SchedulerStatus::Blocked
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }

    #[tokio::test]
    async fn side_effect_ambiguity_blocks_independent_ready_nodes() {
        let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                AmbiguousSideEffectRunner::new(&fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..3 {
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance to ambiguity");
        }
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("ambiguity blocks independent node"),
            SchedulerStatus::Blocked
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_none());
    }

    #[tokio::test]
    async fn side_effect_output_before_confirmation_is_rejected() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                PrematureSideEffectOutputRunner {
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    #[tokio::test]
    async fn side_effect_failure_retryability_must_match_attempt_failure() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                MismatchedSideEffectFailureRunner,
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = SerialTypedScheduler::new(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    struct DeterministicSideEffectRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl DeterministicSideEffectRunner {
        fn new(fixture: &Fixture) -> Self {
            Self {
                cap_kind: side_effect_capability_kind(),
                cap_version: side_effect_capability_version(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            }
        }
    }

    impl ErasedNodeRunner for DeterministicSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key(ctx.attempt_no);
                let phase =
                    side_effect_projection_for_attempt(ctx.projections, ctx.node, ctx.attempt_id)?;
                match phase {
                    None => self.prepare(ctx, ledger),
                    Some(projection)
                        if matches!(
                            projection.phase,
                            store::SideEffectPhase::Claimed { .. }
                                | store::SideEffectPhase::InvocationPrepared { .. }
                        ) =>
                    {
                        let claim = projection.claim.as_ref().expect("claim projection");
                        Ok(ErasedRunnerOutput::new(vec![
                            side_effect_claim_taken_over(&ctx, ledger.clone(), claim, 2),
                            side_effect_prepared(&ctx, ledger.clone(), 1, 2),
                            side_effect_invocation_started(&ctx, ledger, 1, 2),
                        ]))
                    }
                    Some(store::SideEffectProjection {
                        phase:
                            store::SideEffectPhase::InvocationStarted {
                                invocation_epoch, ..
                            }
                            | store::SideEffectPhase::SubmissionUnknown { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc5);
                        let digest = content(0xc4);
                        Ok(ErasedRunnerOutput {
                            required_artifacts: vec![side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Submission,
                            )],
                            payloads: vec![side_effect_submission_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::NotSubmittedProven { invocation_epoch },
                        claim,
                        ..
                    }) => {
                        let claim = claim.as_ref().expect("claim projection");
                        let next_epoch = invocation_epoch + 1;
                        let next_generation = claim.claim_generation + 1;
                        Ok(ErasedRunnerOutput::new(vec![
                            side_effect_claimed(&ctx, ledger.clone(), next_epoch, next_generation),
                            side_effect_prepared(&ctx, ledger.clone(), next_epoch, next_generation),
                            side_effect_invocation_started(
                                &ctx,
                                ledger,
                                next_epoch,
                                next_generation,
                            ),
                        ]))
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::SubmissionObserved { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc7);
                        let digest = content(0xc6);
                        Ok(ErasedRunnerOutput {
                            required_artifacts: vec![side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Receipt,
                            )],
                            payloads: vec![side_effect_receipt_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::ReceiptObserved { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc9);
                        let digest = content(0xc8);
                        Ok(ErasedRunnerOutput {
                            required_artifacts: vec![side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Confirmation,
                            )],
                            payloads: vec![side_effect_confirmation_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::ConfirmationObserved { .. },
                        ..
                    }) => {
                        let artifact = state_output_artifact(
                            ctx.node,
                            ctx.descriptor,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        Ok(ErasedRunnerOutput {
                            required_artifacts: vec![artifact],
                            payloads: terminal_payloads(
                                &ctx,
                                self.output_artifact.clone(),
                                self.output_digest.clone(),
                            ),
                        })
                    }
                    Some(_) => Err(RuntimeError::Blocked(
                        "side-effect fixture blocked".to_owned(),
                    )),
                }
            })
        }
    }

    impl DeterministicSideEffectRunner {
        fn prepare(
            &self,
            ctx: ErasedRunCtx<'_>,
            ledger: events::SideEffectLedgerKey,
        ) -> Result<ErasedRunnerOutput> {
            assert!(ctx.caps.contains(&self.cap_kind, &self.cap_version));
            let intent_artifact_id = artifact(0xc2);
            let intent_hash = content(0xc1);
            Ok(ErasedRunnerOutput {
                required_artifacts: vec![side_effect_artifact(
                    &ctx,
                    intent_artifact_id.clone(),
                    intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                )],
                payloads: vec![
                    events::KernelEventPayload::SideEffectIntentPersisted(
                        events::side_effect::IntentPersisted {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            scope_id: ctx.node.scope_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            ledger_key: ledger.clone(),
                            invocation_epoch: 1,
                            intent_schema_id: ctx.node.config_ref.schema_id.clone(),
                            intent_hash,
                            intent_artifact_id,
                            idempotency_input_schema_id: ctx.node.config_ref.schema_id.clone(),
                            idempotency_input_hash: content(0xc3),
                            idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                .expect("idempotency key"),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                        },
                    ),
                    side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                    side_effect_prepared(&ctx, ledger, 1, 1),
                ],
            })
        }
    }

    struct AmbiguousSideEffectRunner {
        inner: DeterministicSideEffectRunner,
    }

    impl AmbiguousSideEffectRunner {
        fn new(fixture: &Fixture) -> Self {
            Self {
                inner: DeterministicSideEffectRunner::new(fixture),
            }
        }
    }

    impl ErasedNodeRunner for AmbiguousSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key(ctx.attempt_no);
                let phase =
                    side_effect_projection_for_attempt(ctx.projections, ctx.node, ctx.attempt_id)?;
                if matches!(
                    phase.map(|projection| &projection.phase),
                    Some(store::SideEffectPhase::InvocationStarted { .. })
                ) {
                    let artifact_id = artifact(0xcb);
                    let digest = content(0xca);
                    Ok(ErasedRunnerOutput {
                        required_artifacts: vec![side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::AmbiguityEvidence,
                        )],
                        payloads: vec![events::KernelEventPayload::SideEffectAmbiguous(
                            events::side_effect::Ambiguous {
                                spec_hash: ctx.spec_hash.clone(),
                                node_id: ctx.node.node_id.clone(),
                                attempt_id: ctx.attempt_id.clone(),
                                ledger_key: ledger,
                                invocation_epoch: 1,
                                ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                                    .expect("ambiguity code"),
                                evidence_schema_id: ctx.node.config_ref.schema_id.clone(),
                                evidence_hash: digest,
                                evidence_artifact_id: artifact_id,
                            },
                        )],
                    })
                } else {
                    self.inner.run_erased(ctx).await
                }
            })
        }
    }

    struct PrematureSideEffectOutputRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for PrematureSideEffectOutputRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node,
                    ctx.descriptor,
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                Ok(ErasedRunnerOutput {
                    required_artifacts: vec![artifact],
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    struct MismatchedSideEffectFailureRunner;

    impl ErasedNodeRunner for MismatchedSideEffectFailureRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput::new(vec![
                    events::KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
                        spec_hash: ctx.spec_hash.clone(),
                        node_id: ctx.node.node_id.clone(),
                        attempt_id: ctx.attempt_id.clone(),
                        ledger_key: side_effect_ledger_key(ctx.attempt_no),
                        invocation_epoch: 1,
                        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                        retryable: false,
                        error: side_effect_error(false),
                    }),
                    events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                        spec_hash: ctx.spec_hash.clone(),
                        node_id: ctx.node.node_id.clone(),
                        attempt_id: ctx.attempt_id.clone(),
                        retryable: true,
                        error: side_effect_error(true),
                    }),
                ]))
            })
        }
    }

    struct ReadOnlyCorruptStore {
        stream: Vec<store::KernelEventEnvelope>,
        projection: store::ProjectionSnapshot,
    }

    impl store::TypedProjectionRead for ReadOnlyCorruptStore {
        fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
            &self.projection
        }
    }

    impl store::TypedRunEventStore for ReadOnlyCorruptStore {
        fn record_artifact_evidence(
            &mut self,
            _evidence: store::ArtifactEvidenceRef,
        ) -> store::Result<()> {
            Err(store::StoreError::Identity(
                "corrupt test store is read-only".to_owned(),
            ))
        }

        fn append_typed_run_commit(
            &mut self,
            _request: store::TypedCommitRequest,
        ) -> store::Result<store::CommitOutcome> {
            Err(store::StoreError::Identity(
                "corrupt test store is read-only".to_owned(),
            ))
        }

        fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
            self.stream.clone()
        }

        fn expected_next_seq(&self, _run_id: &RunId) -> store::StreamSeq {
            store::StreamSeq::FIRST
        }
    }

    fn run_start_evidence(
        fixture: &Fixture,
        seed_cells: Vec<events::SeedCellRef>,
    ) -> RunStartEvidence {
        RunStartEvidence {
            spec_artifact: spec_artifact(&fixture.runtime_spec),
            config_artifacts: fixture
                .runtime_spec
                .spec()
                .config_refs
                .iter()
                .map(config_artifact)
                .collect(),
            framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework"),
            source_revision: events::SourceRevision::new("test-rev").expect("source"),
            adapter_executables: Vec::new(),
            seed_cells,
        }
    }

    fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> store::ArtifactEvidenceRef {
        let canonical = runtime_spec
            .spec()
            .canonical_json()
            .expect("canonical spec");
        let digest = canonical.content_digest();
        store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        }
    }

    fn config_artifact(config: &spec::ConfigRef) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        }
    }

    fn terminal_payloads(
        ctx: &ErasedRunCtx<'_>,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    ) -> Vec<events::KernelEventPayload> {
        vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                cell_id: ctx.node.output_cell.clone(),
                scope_id: ctx.node.scope_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                semantic_type_id: ctx.descriptor.output_semantic_type_id.clone(),
                schema_id: ctx.descriptor.output_schema_id.clone(),
                value_lineage: ctx.output_cell.value_lineage.clone(),
                artifact_id: output_artifact,
                content_digest: output_digest,
                producer_state_kind: Some(ctx.node.state_kind.clone()),
                producer_state_version: Some(ctx.node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                output_cell_id: ctx.node.output_cell.clone(),
            }),
        ]
    }

    fn state_output_artifact(
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 17,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(descriptor.output_schema_id.clone()),
            semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        }
    }

    fn node_by_output<'a>(fixture: &'a Fixture, cell_id: &CellId) -> &'a spec::NodeSpec {
        fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| &node.output_cell == cell_id)
            .expect("node by output")
    }

    fn append_attempt_start(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_no: u32,
    ) -> AttemptId {
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            attempt_no,
        )
        .expect("attempt id");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-attempt-start:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                    events::StateAttemptStarted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        attempt_no,
                        state_kind: node.state_kind.clone(),
                        state_version: node.state_version.clone(),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append attempt start");
        attempt_id
    }

    fn append_fact(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        fact_key: events::FactKey,
        artifact_id: ArtifactId,
        response_hash: ContentDigest,
    ) {
        let response_schema_id = node.config_ref.schema_id.clone();
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: artifact_id.clone(),
            digest: response_hash.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(response_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactResponse,
        };
        store
            .record_artifact_evidence(evidence.clone())
            .expect("record fact artifact");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-fact:{}:{}:{}",
                    node.node_id, attempt_id, fact_key
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::FactRecorded(
                    events::FactRecorded {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        capability_kind: fixture.cap_kind.clone(),
                        capability_version: fixture.cap_version.clone(),
                        adapter_kind: fixture.adapter_kind.clone(),
                        adapter_version: fixture.adapter_version.clone(),
                        request_schema_id: node.config_ref.schema_id.clone(),
                        request_hash: content(0xd4),
                        response_schema_id,
                        response_hash,
                        fact_key,
                        artifact_id,
                    },
                )],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append fact");
    }

    fn append_terminal(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        artifact_id: ArtifactId,
        output_digest: ContentDigest,
    ) {
        let descriptor = fixture
            .runtime_spec
            .state_descriptor_for_node(node)
            .expect("descriptor");
        let output_cell = fixture
            .runtime_spec
            .cell(&node.output_cell)
            .expect("output cell");
        let evidence =
            state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
        store
            .record_artifact_evidence(evidence.clone())
            .expect("record output artifact");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-terminal:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        cell_id: node.output_cell.clone(),
                        scope_id: node.scope_id.clone(),
                        attempt_id: attempt_id.clone(),
                        semantic_type_id: descriptor.output_semantic_type_id.clone(),
                        schema_id: descriptor.output_schema_id.clone(),
                        value_lineage: output_cell.value_lineage.clone(),
                        artifact_id,
                        content_digest: output_digest,
                        producer_state_kind: Some(node.state_kind.clone()),
                        producer_state_version: Some(node.state_version.clone()),
                    }),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            output_cell_id: node.output_cell.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append terminal");
    }

    fn append_not_submitted_proven(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) {
        let proof_artifact = artifact(0xd5);
        let proof_hash = content(0xd6);
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: proof_artifact.clone(),
            digest: proof_hash.clone(),
            byte_len: 19,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(node.config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::NotSubmittedProof,
        };
        store
            .record_artifact_evidence(evidence.clone())
            .expect("record proof artifact");
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-not-submitted:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                    events::side_effect::NotSubmittedProven {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        ledger_key: side_effect_ledger_key(1),
                        invocation_epoch,
                        proof_schema_id: node.config_ref.schema_id.clone(),
                        proof_hash,
                        proof_artifact_id: proof_artifact,
                    },
                )],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append not-submitted proof");
    }

    fn attempt_started_count(
        store: &store::InMemoryTypedRunStore,
        run_id: &RunId,
        node_id: &NodeId,
    ) -> usize {
        store
            .load_run_stream(run_id)
            .iter()
            .filter(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::StateAttemptStarted(payload)
                        if &payload.node_id == node_id
                )
            })
            .count()
    }

    fn fact_recorded_count(store: &store::InMemoryTypedRunStore) -> usize {
        store
            .projection_snapshot()
            .facts()
            .filter(|(_, fact)| fact.fact_key.as_str() == "reused-fact")
            .count()
    }

    #[test]
    fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec.nodes.reverse();
        let envelope =
            spec::CertifiedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        let runtime = CertifiedRuntimeSpec::new(envelope).expect("runtime");
        assert_eq!(
            runtime.topological_order(),
            fixture.runtime_spec.topological_order()
        );
    }

    fn registered_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        registry
    }

    fn registered_side_effect_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                DeterministicSideEffectRunner::new(fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        registry
    }

    fn binding<R: ErasedNodeRunner + 'static>(
        descriptor_id: DescriptorId,
        factory: &str,
        runner: R,
    ) -> ErasedRunnerBinding {
        let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
        ErasedRunnerBinding::new(
            descriptor_id,
            factory_id.clone(),
            events::ExecutableIdentity {
                factory_id,
                source_revision: events::SourceRevision::new("test-rev").expect("source"),
                cargo_package_name: events::PackageName::new("mfm-test").expect("package"),
                cargo_package_version: events::PackageVersion::new("0.1.0").expect("version"),
                cargo_package_digest: content(0xe1),
                binary_digest: content(0xe2),
                nix_derivation_hash: None,
                nix_output_hash: None,
            },
            Arc::new(runner),
        )
        .expect("runner binding")
    }

    fn fixture() -> Fixture {
        let scope = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0);
        let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, D1);
        let seed_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2);
        let node_a = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D3);
        let cell_a = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D4);
        let node_b = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D5);
        let cell_b = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D6);
        let descriptor_a = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D7);
        let descriptor_b = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
        let semantic =
            SemanticTypeId::new("mfm.test", "value", "1", DigestAlgorithm::Sha256JcsV1, D9)
                .expect("semantic");
        let value_schema =
            SchemaId::new("mfm.test.value", "1", DigestAlgorithm::Sha256JcsV1, DA).expect("schema");
        let input_schema = SchemaId::new("mfm.test.input", "1", DigestAlgorithm::Sha256JcsV1, DB)
            .expect("input schema");
        let config_schema = SchemaId::new("mfm.test.config", "1", DigestAlgorithm::Sha256JcsV1, DC)
            .expect("config schema");
        let public_schema = SchemaId::new("mfm.test.public", "1", DigestAlgorithm::Sha256JcsV1, DD)
            .expect("public schema");
        let effect_kind =
            EffectKind::new("mfm.test", "pure", DigestAlgorithm::Sha256JcsV1, DE).expect("effect");
        let read_effect = EffectKind::new("mfm.test", "read", DigestAlgorithm::Sha256JcsV1, DF)
            .expect("read effect");
        let cap_kind = CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
            .expect("cap kind");
        let cap_version = CapabilityVersion::new("mfm.cap.read_db.v1").expect("cap version");
        let adapter_kind =
            AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
                .expect("adapter kind");
        let adapter_version = AdapterVersion::new("mfm.adapter.v1").expect("adapter version");
        let read_cap = CapabilityDescriptor::new(
            cap_kind.clone(),
            cap_version.clone(),
            CapabilityRole::ReadExternal,
            "read-db",
        )
        .expect("capability");
        let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
        let read_caps = CapabilitySetDescriptor::new(vec![read_cap]).expect("read caps");
        let config_ref = spec::ConfigRef {
            schema_id: config_schema.clone(),
            artifact_id: artifact(0x31),
            digest: content(0x32),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
        };
        let lineage_seed = spec::ValueLineageRef {
            lineage_digest: content(0x41),
        };
        let lineage_a = spec::ValueLineageRef {
            lineage_digest: content(0x42),
        };
        let lineage_b = spec::ValueLineageRef {
            lineage_digest: content(0x43),
        };
        let planning = spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content(0x44),
        };
        let seed_ref = events::SeedCellRef {
            seed_id: seed_id.clone(),
            cell_id: seed_cell.clone(),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            digest: content(0x51),
            seed_artifact: events::ArtifactEvidenceRef {
                artifact_id: artifact(0x52),
                role: events::ArtifactRole::SeedInput,
                schema_id: value_schema.clone(),
                semantic_type_id: Some(semantic.clone()),
                content_digest: content(0x51),
                byte_len: 11,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
        };
        let renderer = spec::RendererDescriptorIdentity {
            descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D1),
            renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer"),
            renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
                .expect("renderer version"),
            public_schema_id: public_schema.clone(),
            canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
                .expect("canonicalizer"),
        };
        let node_a_spec = node_spec(NodeSpecFixture {
            node_id: node_a.clone(),
            descriptor_id: descriptor_a.clone(),
            scope_id: scope.clone(),
            state_name: "mfm.test.state.a",
            state_kind: StateKind::new("mfm.test", "a", DigestAlgorithm::Sha256JcsV1, D2)
                .expect("state a"),
            state_version: StateVersion::new("mfm.test.state.a.v1").expect("state version"),
            effect_kind: effect_kind.clone(),
            config_ref: config_ref.clone(),
            input_schema: input_schema.clone(),
            input_cell: seed_cell.clone(),
            input_lineage: lineage_seed.clone(),
            output_cell: cell_a.clone(),
            output_schema: value_schema.clone(),
            semantic: semantic.clone(),
            caps: no_caps.clone(),
            predecessors: Vec::new(),
            adapter_bindings: Vec::new(),
            planning: planning.clone(),
        });
        let node_b_spec = node_spec(NodeSpecFixture {
            node_id: node_b.clone(),
            descriptor_id: descriptor_b.clone(),
            scope_id: scope.clone(),
            state_name: "mfm.test.state.b",
            state_kind: StateKind::new("mfm.test", "b", DigestAlgorithm::Sha256JcsV1, D3)
                .expect("state b"),
            state_version: StateVersion::new("mfm.test.state.b.v1").expect("state version"),
            effect_kind: read_effect.clone(),
            config_ref: config_ref.clone(),
            input_schema: input_schema.clone(),
            input_cell: cell_a.clone(),
            input_lineage: lineage_a.clone(),
            output_cell: cell_b.clone(),
            output_schema: value_schema.clone(),
            semantic: semantic.clone(),
            caps: read_caps.clone(),
            predecessors: vec![node_a.clone()],
            adapter_bindings: vec![spec::AdapterBinding {
                adapter_kind: adapter_kind.clone(),
                adapter_version: adapter_version.clone(),
                binding_digest: None,
            }],
            planning: planning.clone(),
        });
        let spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: spec::AuthoringProvenance::StateComposition {
                descriptor: spec::CompositionDescriptor {
                    descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D4),
                    name: "mfm.test.composition".to_owned(),
                    version: "mfm.test.composition.v1".to_owned(),
                },
                config_hash: content(0x60),
            },
            scopes: vec![spec::ScopeSpec {
                scope_id: scope.clone(),
                parent_scope_id: None,
                stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
                planning_lineage: planning.clone(),
            }],
            seeds: vec![spec::SeedSpec {
                seed_id: seed_id.clone(),
                seed_key: spec::StableAuthorKey::new("launch").expect("seed key"),
                cell_id: seed_cell.clone(),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                required_digest: Some(content(0x51)),
            }],
            descriptor_identities: vec![
                spec::DescriptorIdentity::State(Box::new(state_descriptor(
                    &node_a_spec,
                    descriptor_a.clone(),
                    "mfm.test.state.a",
                    effect_kind,
                    no_caps,
                    "pure",
                ))),
                spec::DescriptorIdentity::State(Box::new(state_descriptor(
                    &node_b_spec,
                    descriptor_b.clone(),
                    "mfm.test.state.b",
                    read_effect,
                    read_caps,
                    "read",
                ))),
                spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
            ],
            config_refs: vec![config_ref.clone()],
            nodes: vec![node_b_spec.clone(), node_a_spec.clone()],
            cells: vec![
                spec::CellSpec {
                    cell_id: seed_cell,
                    producer: spec::CellProducer::Seed(seed_id),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_seed.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: cell_a.clone(),
                    producer: spec::CellProducer::Node(node_a.clone()),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_a.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: cell_b.clone(),
                    producer: spec::CellProducer::Node(node_b.clone()),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_b.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
            ],
            value_lineages: vec![
                spec::ValueLineage {
                    lineage_ref: lineage_seed.clone(),
                    scope_id: scope.clone(),
                    producer: spec::CellProducer::Seed(SeedId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        D1,
                    )),
                    input_cells: Vec::new(),
                    config_ref_digest: None,
                    planning_lineage: planning.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::Source,
                },
                spec::ValueLineage {
                    lineage_ref: lineage_a.clone(),
                    scope_id: scope.clone(),
                    producer: spec::CellProducer::Node(node_a.clone()),
                    input_cells: vec![CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2)],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
                spec::ValueLineage {
                    lineage_ref: lineage_b.clone(),
                    scope_id: scope,
                    producer: spec::CellProducer::Node(node_b.clone()),
                    input_cells: vec![cell_a.clone()],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning,
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
            ],
            planning_lineage: Vec::new(),
            public_outputs: spec::PublicOutputSpec {
                public_schema_id: public_schema,
                outputs: vec![spec::PublicOutputCell {
                    public_field_path: spec::PublicFieldPath::new("result").expect("field"),
                    cell_id: cell_b.clone(),
                    producer: spec::CellProducer::Node(node_b.clone()),
                    scope_id: ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_b.clone(),
                    required_terminal: spec::RequiredTerminal::ProducedOnly,
                }],
                renderer_descriptor: renderer,
            },
        })
        .expect("typed spec");
        let envelope =
            spec::CertifiedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
                .expect("envelope");
        let runtime_spec = CertifiedRuntimeSpec::new(envelope).expect("runtime spec");
        Fixture {
            runtime_spec,
            run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, D5),
            seed_ref,
            descriptor_a,
            descriptor_b,
            cell_a,
            cell_b,
            cap_kind,
            cap_version,
            adapter_kind,
            adapter_version,
        }
    }

    fn fixture_with_first_managed_write_state() -> Fixture {
        let mut fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let managed_effect = EffectKind::new(
            "mfm.test",
            "managed-write",
            DigestAlgorithm::Sha256JcsV1,
            D8,
        )
        .expect("managed effect");
        let managed_cap = CapabilityDescriptor::new(
            CapabilityKind::new(
                "mfm.test",
                "managed-store",
                DigestAlgorithm::Sha256JcsV1,
                D9,
            )
            .expect("managed cap kind"),
            CapabilityVersion::new("mfm.cap.managed_store.v1").expect("managed cap version"),
            CapabilityRole::ManagedPlatformWrite,
            "managed-store",
        )
        .expect("managed cap");
        let managed_caps = CapabilitySetDescriptor::new(vec![managed_cap]).expect("managed caps");
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_a {
                node.effect_kind = managed_effect.clone();
                node.capability_bindings = managed_caps.clone();
            }
        }
        for descriptor in &mut envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                if identity.descriptor_id == fixture.descriptor_a {
                    identity.effect_kind = managed_effect.clone();
                    identity.effect_class = "managed-write".to_owned();
                    identity.effect_name = "managed-write".to_owned();
                    identity.capabilities = managed_caps.clone();
                    identity.runner = "managed-write".to_owned();
                }
            }
        }
        let envelope =
            spec::CertifiedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec = CertifiedRuntimeSpec::new(envelope).expect("runtime spec");
        fixture
    }

    fn fixture_with_first_side_effect_state() -> Fixture {
        let mut fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let side_effect =
            EffectKind::new("mfm.test", "side-effect", DigestAlgorithm::Sha256JcsV1, D8)
                .expect("side-effect");
        let side_effect_cap = CapabilityDescriptor::new(
            side_effect_capability_kind(),
            side_effect_capability_version(),
            CapabilityRole::ExternalMutationAuthority,
            "external-mutation",
        )
        .expect("side-effect cap");
        let side_effect_caps =
            CapabilitySetDescriptor::new(vec![side_effect_cap]).expect("side-effect caps");
        let contract_digest = content(0x88);
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_a {
                node.effect_kind = side_effect.clone();
                node.capability_bindings = side_effect_caps.clone();
                node.adapter_bindings = vec![spec::AdapterBinding {
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                    binding_digest: None,
                }];
                node.side_effect = Some(spec::SideEffectContractSpec {
                    contract_digest: contract_digest.clone(),
                });
            }
        }
        for descriptor in &mut envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                if identity.descriptor_id == fixture.descriptor_a {
                    identity.effect_kind = side_effect.clone();
                    identity.effect_class = "sidefx".to_owned();
                    identity.effect_name = "sidefx".to_owned();
                    identity.capabilities = side_effect_caps.clone();
                    identity.runner = "sidefx".to_owned();
                    identity.side_effect_contract_digest = Some(contract_digest.clone());
                }
            }
        }
        let envelope =
            spec::CertifiedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec = CertifiedRuntimeSpec::new(envelope).expect("runtime spec");
        fixture
    }

    fn fixture_with_independent_second_node_and_first_side_effect_state() -> Fixture {
        let mut fixture = fixture_with_first_side_effect_state();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let seed_cell = envelope
            .spec
            .cells
            .iter()
            .find(|cell| cell.cell_id == fixture.seed_ref.cell_id)
            .expect("seed cell")
            .clone();
        let node_b_id = envelope
            .spec
            .nodes
            .iter()
            .find(|node| node.descriptor_id == fixture.descriptor_b)
            .expect("node b")
            .node_id
            .clone();
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_b {
                node.input_bindings.root =
                    spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: spec::PublicFieldPath::new("input").expect("field"),
                        cell_id: seed_cell.cell_id.clone(),
                        semantic_type_id: seed_cell.semantic_type_id.clone(),
                        schema_id: seed_cell.schema_id.clone(),
                        required_terminal: spec::RequiredTerminal::ProducedOnly,
                        value_lineage: seed_cell.value_lineage.clone(),
                    }));
                node.deterministic_predecessors.clear();
            }
        }
        for lineage in &mut envelope.spec.value_lineages {
            if lineage.producer == spec::CellProducer::Node(node_b_id.clone()) {
                lineage.input_cells = vec![seed_cell.cell_id.clone()];
            }
        }
        let envelope =
            spec::CertifiedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec = CertifiedRuntimeSpec::new(envelope).expect("runtime spec");
        fixture
    }

    struct NodeSpecFixture {
        node_id: NodeId,
        descriptor_id: DescriptorId,
        scope_id: ScopeId,
        state_name: &'static str,
        state_kind: StateKind,
        state_version: StateVersion,
        effect_kind: EffectKind,
        config_ref: spec::ConfigRef,
        input_schema: SchemaId,
        input_cell: CellId,
        input_lineage: spec::ValueLineageRef,
        output_cell: CellId,
        output_schema: SchemaId,
        semantic: SemanticTypeId,
        caps: CapabilitySetDescriptor,
        predecessors: Vec<NodeId>,
        adapter_bindings: Vec<spec::AdapterBinding>,
        planning: spec::PlanningLineage,
    }

    fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
        spec::NodeSpec {
            node_id: fixture.node_id,
            stable_key: spec::StableAuthorKey::new(
                fixture.state_name.rsplit('.').next().expect("state key"),
            )
            .expect("stable key"),
            scope_id: fixture.scope_id,
            state_kind: fixture.state_kind,
            state_version: fixture.state_version,
            descriptor_id: fixture.descriptor_id,
            config_ref: fixture.config_ref,
            input_bindings: spec::InputBindingSpec {
                input_schema_id: fixture.input_schema,
                input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D6),
                root: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: spec::PublicFieldPath::new("input").expect("field"),
                    cell_id: fixture.input_cell,
                    semantic_type_id: fixture.semantic.clone(),
                    schema_id: fixture.output_schema.clone(),
                    required_terminal: spec::RequiredTerminal::ProducedOnly,
                    value_lineage: fixture.input_lineage,
                })),
                digest: content(0x73),
            },
            output_cell: fixture.output_cell,
            effect_kind: fixture.effect_kind,
            capability_bindings: fixture.caps,
            adapter_bindings: fixture.adapter_bindings,
            side_effect: None,
            framework: None,
            planning_lineage: fixture.planning,
            deterministic_predecessors: fixture.predecessors,
        }
    }

    fn state_descriptor(
        node: &spec::NodeSpec,
        descriptor_id: DescriptorId,
        name: &str,
        effect_kind: EffectKind,
        capabilities: CapabilitySetDescriptor,
        runner: &str,
    ) -> spec::StateDescriptorIdentity {
        spec::StateDescriptorIdentity {
            descriptor_id,
            name: name.to_owned(),
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
            config_schema_id: node.config_ref.schema_id.clone(),
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            output_schema_id: node
                .input_bindings
                .root
                .clone()
                .first_schema_or(node.config_ref.schema_id.clone()),
            output_semantic_type_id: match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.semantic_type_id.clone(),
                _ => panic!("test input"),
            },
            effect_kind,
            effect_class: runner.to_owned(),
            effect_name: runner.to_owned(),
            effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
            capabilities,
            runner: runner.to_owned(),
            side_effect_contract_digest: None,
        }
    }

    trait FirstSchema {
        fn first_schema_or(&self, fallback: SchemaId) -> SchemaId;
    }

    impl FirstSchema for spec::InputBindingNodeSpec {
        fn first_schema_or(&self, fallback: SchemaId) -> SchemaId {
            match self {
                spec::InputBindingNodeSpec::Cell(cell) => cell.schema_id.clone(),
                _ => fallback,
            }
        }
    }

    fn content(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn artifact(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn side_effect_capability_kind() -> CapabilityKind {
        CapabilityKind::new(
            "mfm.test",
            "external-mutation",
            DigestAlgorithm::Sha256JcsV1,
            D9,
        )
        .expect("side-effect cap kind")
    }

    fn side_effect_capability_version() -> CapabilityVersion {
        CapabilityVersion::new("mfm.cap.external_mutation.v1").expect("side-effect cap version")
    }

    fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
        events::SideEffectLedgerKey::new(format!("ledger-{attempt_no}")).expect("ledger key")
    }

    fn side_effect_claim_owner(attempt_no: u32, generation: u32) -> events::RunnerInvocationId {
        events::RunnerInvocationId::new(format!("owner-{attempt_no}-{generation}"))
            .expect("claim owner")
    }

    fn side_effect_fencing_token(
        attempt_no: u32,
        generation: u32,
    ) -> events::side_effect::ClaimFencingToken {
        events::side_effect::ClaimFencingToken::new(format!("token-{attempt_no}-{generation}"))
            .expect("fencing token")
    }

    fn side_effect_artifact(
        ctx: &ErasedRunCtx<'_>,
        artifact_id: ArtifactId,
        digest: ContentDigest,
        role: events::ArtifactRole,
    ) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 19,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(ctx.node.config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(ctx.node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: role,
        }
    }

    fn side_effect_claimed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
            invocation_epoch,
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
        })
    }

    fn side_effect_claim_taken_over(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        previous: &store::SideEffectClaimProjection,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger,
            previous_claim_owner: previous.claim_owner.clone(),
            new_claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
            invocation_epoch: previous.invocation_epoch,
            previous_claim_generation: previous.claim_generation,
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
        })
    }

    fn side_effect_prepared(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectInvocationPrepared(
            events::side_effect::InvocationPrepared {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger,
                invocation_epoch,
                claim_generation,
                claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
                prepared_artifact_id: None,
                prepared_hash: None,
            },
        )
    }

    fn side_effect_invocation_started(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectInvocationStarted(
            events::side_effect::InvocationStarted {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger,
                invocation_epoch,
                claim_owner: side_effect_claim_owner(ctx.attempt_no, claim_generation),
                claim_generation,
                claim_fencing_token: side_effect_fencing_token(ctx.attempt_no, claim_generation),
            },
        )
    }

    fn side_effect_submission_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectSubmissionObserved(
            events::side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger,
                invocation_epoch,
                submission_schema_id: ctx.node.config_ref.schema_id.clone(),
                submission_hash: digest,
                submission_artifact_id: artifact_id,
            },
        )
    }

    fn side_effect_receipt_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectReceiptObserved(
            events::side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger,
                invocation_epoch,
                receipt_schema_id: ctx.node.config_ref.schema_id.clone(),
                receipt_hash: digest,
                receipt_artifact_id: artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
            },
        )
    }

    fn side_effect_confirmation_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectConfirmationObserved(
            events::side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger,
                invocation_epoch,
                confirmation_schema_id: ctx.node.config_ref.schema_id.clone(),
                confirmation_hash: digest,
                confirmation_artifact_id: artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
            },
        )
    }

    fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
        events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        }
    }
}
