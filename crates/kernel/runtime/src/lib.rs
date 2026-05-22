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
    /// Store-owned projection snapshot observed before the attempt.
    pub projections: &'a store::ProjectionSnapshot,
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
        let Some(node) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt(store, runtime_spec, run_id, &view, node)
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
        node: &spec::NodeSpec,
    ) -> Result<()> {
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
        let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;

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

        let latest_projection = store.projection_snapshot().clone();
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
                projections: &latest_projection,
            })
            .await?;
        validate_runner_output(runtime_spec, node, &attempt_id, &caps, &output)?;
        for artifact in &output.required_artifacts {
            store.record_artifact_evidence(artifact.clone())?;
        }
        let terminal_request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(format!(
                "attempt-terminal:{}:{}",
                node.node_id, attempt_id
            ))?,
            payloads: output.payloads,
            required_artifacts: output.required_artifacts,
            preconditions: store::CommitPreconditions {
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
            },
        };
        store.append_typed_run_commit(terminal_request)?;
        Ok(())
    }
}

fn next_runnable_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
) -> Result<Option<&'a spec::NodeSpec>> {
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if view.projections.cell_terminal(&node.output_cell).is_some() {
            continue;
        }
        if has_started_attempt(&view.projections, &node.node_id) {
            continue;
        }
        if node_inputs_ready(runtime_spec, node, view)? {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

fn has_started_attempt(projections: &store::ProjectionSnapshot, node_id: &NodeId) -> bool {
    projections
        .attempts()
        .any(|((attempt_node_id, _), _)| attempt_node_id == node_id)
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

fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::RunCompleted(_)
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
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_historical_produced_cell(runtime_spec, projections, event, payload)?;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
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
                let fact = projections
                    .fact(&payload.node_id, &payload.attempt_id, &payload.fact_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "fact {} for node {} attempt {} is not projected",
                            payload.fact_key, payload.node_id, payload.attempt_id
                        ))
                    })?;
                if fact.event_id != *event.event_id()
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
                return Err(RuntimeError::InvalidRunStream(
                    "side-effect events are not accepted before the side-effect scheduler protocol is enabled"
                        .to_owned(),
                ));
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
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned side-effect payload before the side-effect scheduler protocol is enabled",
                    node.node_id
                )));
            }
        }
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

    #[tokio::test]
    async fn replay_rejects_fact_without_projected_attempt() {
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
        store
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
            .expect("append forged fact");
        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
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
}
