#![warn(missing_docs)]
//! Certification contracts for MFM typed execution specs.
//!
//! This crate lowers typed program drafts into `mfm_spec::v1` specs and validates
//! that a v1 typed execution spec is the only semantic runtime contract.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilitySet, CapabilitySetDescriptor, NoCaps};
use mfm_effects::{ApplySideEffect, EffectClass, EffectSpec, ManagedPlatformWrite, Pure};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind,
    NodeId, OperationInstanceId, SchemaId, ScopeId, SemanticTypeId, StateKind, StateVersion,
};
use mfm_program as program;
use mfm_spec::v1 as spec;

/// Result type for typed certification.
pub type Result<T> = std::result::Result<T, CertifyError>;

/// Problem taxonomy class rejected by typed certification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProblemClass {
    /// Duplicate, missing, cyclic, or otherwise impossible graph structure.
    InvalidTopology,
    /// Producer/consumer interface metadata does not match the referenced cell.
    InvalidInterfaceWiring,
    /// Effect, capability, descriptor, runner, or side-effect transition evidence is invalid.
    InvalidSemanticTransition,
    /// Canonical data, descriptor, config, dynamic ordering, or persisted shape evidence is invalid.
    InvalidDataShape,
    /// Value lineage, scope, producer, or domain meaning evidence is invalid.
    InvalidDataMeaning,
    /// Public output, render node, or terminal-output evidence is invalid.
    InvalidTerminalShape,
}

impl ProblemClass {
    /// Returns the stable summary key used by typed-kernel contract checks.
    pub const fn summary_key(self) -> &'static str {
        match self {
            Self::InvalidTopology => "invalid_topology_rejected",
            Self::InvalidInterfaceWiring => "invalid_interface_wiring_rejected",
            Self::InvalidSemanticTransition => "invalid_semantic_transition_rejected",
            Self::InvalidDataShape => "invalid_data_shape_rejected",
            Self::InvalidDataMeaning => "invalid_data_meaning_rejected",
            Self::InvalidTerminalShape => "invalid_terminal_shape_rejected",
        }
    }
}

/// Certification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertifyError {
    /// A typed-core problem class was rejected.
    Problem {
        /// Rejected problem class.
        class: ProblemClass,
        /// Stable diagnostic.
        message: String,
    },
    /// Lowering a program draft failed before v1 validation.
    Lowering(String),
    /// Canonicalization or hashing failed.
    Canonical(String),
    /// Spec contract validation failed.
    Spec(String),
}

impl CertifyError {
    /// Returns this error's problem class when it is a taxonomy rejection.
    pub fn problem_class(&self) -> Option<ProblemClass> {
        match self {
            Self::Problem { class, .. } => Some(*class),
            Self::Lowering(_) | Self::Canonical(_) | Self::Spec(_) => None,
        }
    }
}

impl fmt::Display for CertifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Problem { class, message } => {
                write!(f, "{class:?}: {message}")
            }
            Self::Lowering(message) => write!(f, "typed lowering failed: {message}"),
            Self::Canonical(message) => write!(f, "canonicalization failed: {message}"),
            Self::Spec(message) => write!(f, "typed spec failed: {message}"),
        }
    }
}

impl std::error::Error for CertifyError {}

/// Certified typed spec ready to become the runtime contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedTypedSpec {
    /// Certified spec envelope.
    pub envelope: spec::CertifiedSpecEnvelope,
}

/// Registry authority used when certifying an already-lowered typed spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CertificationRegistry {
    states: BTreeMap<String, spec::StateDescriptorIdentity>,
    operations: BTreeMap<String, spec::OperationDescriptorIdentity>,
}

impl CertificationRegistry {
    /// Creates an empty certification registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a framework-validated registered state descriptor to this registry.
    pub fn register_state<S>(&mut self, registered: &program::RegisteredState<S>) -> Result<()>
    where
        S: program::StateSpec,
    {
        self.insert_state(state_descriptor_identity_from_registered(
            registered.descriptor(),
            registered.runner(),
        )?)
    }

    /// Adds a framework-validated registered operation descriptor to this registry.
    pub fn register_operation<O>(
        &mut self,
        registered: &program::RegisteredOperation<O>,
    ) -> Result<()>
    where
        O: program::Operation,
    {
        self.insert_operation(operation_descriptor_identity_from_registered(
            registered.descriptor(),
        ))
    }

    fn from_program_draft(draft: &program::TypedProgramDraft) -> Result<Self> {
        let mut registry = Self::new();
        for node in draft.state_nodes() {
            registry.insert_state(state_descriptor_identity_from_program(node)?)?;
        }
        for frame in draft.operation_lineage() {
            registry.insert_operation(operation_descriptor_identity_from_program(frame))?;
        }
        Ok(registry)
    }

    fn insert_state(&mut self, descriptor: spec::StateDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.states.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered state descriptor {key}"),
                ));
            }
        } else {
            self.states.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_operation(&mut self, descriptor: spec::OperationDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.operations.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered operation descriptor {key}"),
                ));
            }
        } else {
            self.operations.insert(key, descriptor);
        }
        Ok(())
    }
}

/// Lowers and certifies a typed program draft.
pub fn certify_program_draft(draft: &program::TypedProgramDraft) -> Result<CertifiedTypedSpec> {
    let registry = CertificationRegistry::from_program_draft(draft)?;
    let spec = lower_program_draft_with_registry(draft, &registry)?;
    certify_typed_spec(spec, &registry)
}

/// Certifies an already-lowered v1 typed execution spec.
pub fn certify_typed_spec(
    spec: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    validate_typed_spec(&spec, registry)?;
    let envelope = spec::CertifiedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    envelope
        .verify_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(CertifiedTypedSpec { envelope })
}

/// Lowers a typed program draft into a v1 spec without skipping validation.
pub fn lower_program_draft(draft: &program::TypedProgramDraft) -> Result<spec::TypedExecutionSpec> {
    let registry = CertificationRegistry::from_program_draft(draft)?;
    lower_program_draft_with_registry(draft, &registry)
}

fn lower_program_draft_with_registry(
    draft: &program::TypedProgramDraft,
    registry: &CertificationRegistry,
) -> Result<spec::TypedExecutionSpec> {
    let mut lowerer = DraftLowerer::new(draft)?;
    let lowered = lowerer.lower()?;
    validate_typed_spec(&lowered, registry)?;
    Ok(lowered)
}

fn problem(class: ProblemClass, message: impl Into<String>) -> CertifyError {
    CertifyError::Problem {
        class,
        message: message.into(),
    }
}

fn lower(message: impl Into<String>) -> CertifyError {
    CertifyError::Lowering(message.into())
}

fn canonical(message: impl Into<String>) -> CertifyError {
    CertifyError::Canonical(message.into())
}

#[derive(Debug, Clone)]
struct CellInfo {
    producer: spec::CellProducer,
    semantic_type_id: SemanticTypeId,
    schema_id: SchemaId,
    value_lineage: spec::ValueLineageRef,
}

struct DraftLowerer<'a> {
    draft: &'a program::TypedProgramDraft,
    cells: Vec<spec::CellSpec>,
    cell_info: BTreeMap<String, CellInfo>,
    config_refs: BTreeMap<String, spec::ConfigRef>,
    descriptor_identities: BTreeMap<String, spec::DescriptorIdentity>,
    value_lineages: BTreeMap<String, spec::ValueLineage>,
    nodes: Vec<spec::NodeSpec>,
}

impl<'a> DraftLowerer<'a> {
    fn new(draft: &'a program::TypedProgramDraft) -> Result<Self> {
        Ok(Self {
            draft,
            cells: Vec::new(),
            cell_info: BTreeMap::new(),
            config_refs: BTreeMap::new(),
            descriptor_identities: BTreeMap::new(),
            value_lineages: BTreeMap::new(),
            nodes: Vec::new(),
        })
    }

    fn lower(&mut self) -> Result<spec::TypedExecutionSpec> {
        let scopes = self.lower_scopes()?;
        let seeds = self.lower_seeds()?;
        self.lower_state_nodes()?;
        self.lower_bridge_nodes()?;
        let public_outputs = self.lower_public_outputs()?;
        let planning_lineage = self.lower_operation_lineage()?;

        spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: self.authoring_provenance()?,
            scopes,
            seeds,
            descriptor_identities: self.descriptor_identities.values().cloned().collect(),
            config_refs: self.config_refs.values().cloned().collect(),
            nodes: self.nodes.clone(),
            cells: self.cells.clone(),
            value_lineages: self.value_lineages.values().cloned().collect(),
            planning_lineage,
            public_outputs,
        })
        .map_err(|error| CertifyError::Spec(error.to_string()))
    }

    fn lower_scopes(&self) -> Result<Vec<spec::ScopeSpec>> {
        self.draft
            .scopes()
            .iter()
            .map(|scope| {
                Ok(spec::ScopeSpec {
                    scope_id: scope.scope_id.clone(),
                    parent_scope_id: scope.parent_scope_id.clone(),
                    stable_key: stable_author_key(scope.key.as_str())?,
                    planning_lineage: lower_planning_lineage(&scope.planning_lineage),
                })
            })
            .collect()
    }

    fn lower_seeds(&mut self) -> Result<Vec<spec::SeedSpec>> {
        let mut lowered = Vec::new();
        for seed in self.draft.seeds() {
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(seed.value_lineage.digest()),
                scope_id: seed.scope_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: empty_planning_lineage()?,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::Source,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: seed.cell_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                value_lineage: lineage_ref(seed.value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            })?;
            lowered.push(spec::SeedSpec {
                seed_id: seed.seed_id.clone(),
                seed_key: stable_author_key(seed.key.as_str())?,
                cell_id: seed.cell_id.clone(),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                required_digest: Some(seed.content_digest.clone()),
            });
        }
        Ok(lowered)
    }

    fn lower_state_nodes(&mut self) -> Result<()> {
        for node in self.draft.state_nodes() {
            let config_ref = self.config_ref(&node.config)?;
            let input_bindings = lower_input_binding(&node.input)?;
            let input_cells = collect_input_cells(&input_bindings.root);
            let planning_lineage = lower_planning_lineage(&node.planning_lineage);
            let domain_keys = node
                .output_domain_keys
                .iter()
                .map(lower_domain_key_ref)
                .collect::<Vec<_>>();
            let output_lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(node.output_value_lineage.digest()),
                scope_id: node.scope_id.clone(),
                producer: spec::CellProducer::Node(node.node_id.clone()),
                input_cells: sorted_cell_ids(input_cells.clone()),
                config_ref_digest: Some(node.config.config_ref_digest.clone()),
                planning_lineage: planning_lineage.clone(),
                domain_keys,
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            };
            self.insert_value_lineage(output_lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: node.output_cell_id.clone(),
                producer: spec::CellProducer::Node(node.node_id.clone()),
                scope_id: node.scope_id.clone(),
                semantic_type_id: node.output_semantic_type_id.clone(),
                schema_id: node.output_schema_id.clone(),
                value_lineage: lineage_ref(node.output_value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            })?;
            self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
                state_descriptor_identity_from_program(node)?,
            )))?;
            let side_effect = node.side_effect_contract_digest.as_ref().map(|digest| {
                spec::SideEffectContractSpec {
                    contract_digest: digest.clone(),
                }
            });
            self.nodes.push(spec::NodeSpec {
                node_id: node.node_id.clone(),
                stable_key: stable_author_key(node.key.as_str())?,
                scope_id: node.scope_id.clone(),
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
                descriptor_id: node.state_descriptor_id.clone(),
                config_ref,
                input_bindings,
                output_cell: node.output_cell_id.clone(),
                effect_kind: node.effect_kind.clone(),
                capability_bindings: node.capability_bindings.clone(),
                adapter_bindings: node
                    .adapter_bindings
                    .iter()
                    .map(|binding| spec::AdapterBinding {
                        adapter_kind: binding.adapter_kind.clone(),
                        adapter_version: binding.adapter_version.clone(),
                        binding_digest: None,
                    })
                    .collect(),
                side_effect,
                framework: None,
                planning_lineage,
                deterministic_predecessors: self.predecessors_for_inputs(&input_cells)?,
            });
        }
        Ok(())
    }

    fn lower_bridge_nodes(&mut self) -> Result<()> {
        for bridge in self.draft.bridge_nodes() {
            let source = self
                .cell_info
                .get(bridge.source_cell_id.as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "bridge source cell {} has no producer",
                            bridge.source_cell_id
                        ),
                    )
                })?;
            let config_ref = framework_config_ref("bridge_same_value", &bridge.node_id)?;
            self.insert_config_ref(config_ref.clone())?;
            let input_binding =
                single_cell_input_binding(source.clone(), bridge.source_cell_id.clone(), "source")?;
            let planning_lineage = lower_planning_lineage(&bridge.planning_lineage);
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(bridge.target_value_lineage.digest()),
                scope_id: bridge.target_scope_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                input_cells: vec![bridge.source_cell_id.clone()],
                config_ref_digest: None,
                planning_lineage: planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::SameValueBridge,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: bridge.target_cell_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                scope_id: bridge.target_scope_id.clone(),
                semantic_type_id: bridge.semantic_type_id.clone(),
                schema_id: bridge.schema_id.clone(),
                value_lineage: lineage_ref(bridge.target_value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            })?;
            let descriptor = framework_bridge_descriptor(
                &bridge.schema_id,
                &bridge.semantic_type_id,
                &config_ref.schema_id,
                &input_binding.input_schema_id,
            )?;
            self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
                descriptor.clone(),
            )))?;
            self.nodes.push(spec::NodeSpec {
                node_id: bridge.node_id.clone(),
                stable_key: stable_author_key(bridge.key.as_str())?,
                scope_id: bridge.target_scope_id.clone(),
                state_kind: descriptor.state_kind,
                state_version: descriptor.state_version,
                descriptor_id: descriptor.descriptor_id,
                config_ref,
                input_bindings: input_binding,
                output_cell: bridge.target_cell_id.clone(),
                effect_kind: descriptor.effect_kind,
                capability_bindings: descriptor.capabilities,
                adapter_bindings: Vec::new(),
                side_effect: None,
                framework: Some(spec::FrameworkNodeSpec::Bridge(spec::BridgeNodeSpec {
                    bridge_kind: lower_bridge_kind(bridge.bridge_kind),
                    source_scope_id: bridge.source_scope_id.clone(),
                    target_scope_id: bridge.target_scope_id.clone(),
                    source_cell_id: bridge.source_cell_id.clone(),
                    target_cell_id: bridge.target_cell_id.clone(),
                    semantic_type_id: bridge.semantic_type_id.clone(),
                    schema_id: bridge.schema_id.clone(),
                    policy: lower_bridge_policy(bridge.policy),
                    provenance: spec::BridgeProvenance::FrameworkChildScopeV1,
                })),
                planning_lineage,
                deterministic_predecessors: self
                    .predecessors_for_inputs(std::slice::from_ref(&bridge.source_cell_id))?,
            });
        }
        Ok(())
    }

    fn lower_public_outputs(&mut self) -> Result<spec::PublicOutputSpec> {
        let mut outputs = Vec::new();
        for output in self.draft.public_output_spec().outputs() {
            let cell = output.cell();
            let info = self
                .cell_info
                .get(cell.cell_id().as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTerminalShape,
                        format!("public output cell {} has no producer", cell.cell_id()),
                    )
                })?;
            outputs.push(spec::PublicOutputCell {
                public_field_path: spec::PublicFieldPath::new(output.public_field_path().as_str())
                    .map_err(|error| lower(error.to_string()))?,
                cell_id: cell.cell_id().clone(),
                producer: info.producer,
                scope_id: cell.scope_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
            });
        }
        let renderer_descriptor =
            renderer_descriptor(self.draft.public_output_spec().public_schema_id())?;
        self.insert_descriptor(spec::DescriptorIdentity::Renderer(Box::new(
            renderer_descriptor.clone(),
        )))?;
        let public_outputs = spec::PublicOutputSpec {
            public_schema_id: self.draft.public_output_spec().public_schema_id().clone(),
            outputs,
            renderer_descriptor,
        };
        self.lower_public_output_render_node(&public_outputs)?;
        Ok(public_outputs)
    }

    fn lower_public_output_render_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
    ) -> Result<()> {
        let output_spec_digest = public_outputs
            .digest()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let node_id = render_node_id(
            self.draft.root_scope_id(),
            self.draft.public_output_spec().key().as_str(),
            &output_spec_digest,
        )?;
        let semantic_type_id = spec::public_output_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::public_output_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("public_output_render", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let required_cells = public_outputs.outputs.clone();
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            &required_cells
                .iter()
                .map(|output| output.cell_id.clone())
                .collect::<Vec<_>>(),
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        let input_root = spec::InputBindingNodeSpec::Struct(
            required_cells
                .iter()
                .map(|output| spec::NamedInputBindingSpec {
                    field_path: output.public_field_path.clone(),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: output.public_field_path.clone(),
                        cell_id: output.cell_id.clone(),
                        semantic_type_id: output.semantic_type_id.clone(),
                        schema_id: output.schema_id.clone(),
                        required_terminal: output.required_terminal,
                        value_lineage: output.value_lineage.clone(),
                    })),
                })
                .collect(),
        );
        let input_binding = spec::InputBindingSpec {
            input_schema_id: public_outputs.public_schema_id.clone(),
            input_descriptor_id: descriptor_id_json(serde_json::json!({
                "framework": "public_output_render_input",
                "public_schema_id": public_outputs.public_schema_id.as_str(),
            }))?,
            digest: content_digest_json(input_node_json(&input_root))?,
            root: input_root,
        };
        let descriptor = framework_render_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: sorted_cell_ids(
                required_cells
                    .iter()
                    .map(|output| output.cell_id.clone())
                    .collect(),
            ),
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::PublicOutputArtifact,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        let input_cell_ids = required_cells
            .iter()
            .map(|output| output.cell_id.clone())
            .collect::<Vec<_>>();
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key: stable_author_key(self.draft.public_output_spec().key().as_str())?,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell,
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
                spec::PublicOutputRenderNodeSpec {
                    public_schema_id: public_outputs.public_schema_id.clone(),
                    output_spec_digest,
                    renderer_descriptor: public_outputs.renderer_descriptor.clone(),
                    required_cells,
                },
            )),
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cell_ids)?,
        });
        Ok(())
    }

    fn lower_operation_lineage(&mut self) -> Result<Vec<spec::OperationLineageFrameSpec>> {
        let mut frames = Vec::new();
        for frame in self.draft.operation_lineage() {
            self.config_ref(&frame.config)?;
            let input = lower_operation_input_binding(&frame.input)?;
            self.insert_descriptor(spec::DescriptorIdentity::Operation(Box::new(
                operation_descriptor_identity_from_program(frame),
            )))?;
            for output in &frame.output_handles {
                if !self.cell_info.contains_key(output.cell_id().as_str()) {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "operation frame {} returned unknown cell {}",
                            frame.operation_instance_id,
                            output.cell_id()
                        ),
                    ));
                }
            }
            frames.push(spec::OperationLineageFrameSpec {
                operation_instance_id: frame.operation_instance_id.clone(),
                operation_key: stable_author_key(frame.key.as_str())?,
                scope_id: frame.scope_id.clone(),
                operation_descriptor_id: frame.operation_descriptor_id.clone(),
                config_ref_digest: frame.config.config_ref_digest.clone(),
                input_bindings: input.clone(),
                input_binding_digest: input.digest,
                parent_planning_lineage: lower_planning_lineage(&frame.parent_operation_lineage),
                output_cells: frame
                    .output_handles
                    .iter()
                    .map(|handle| handle.cell_id().clone())
                    .collect(),
                lineage_digest: frame.lineage_digest.clone(),
            });
        }
        Ok(frames)
    }

    fn config_ref(&mut self, binding: &program::ConfigBindingSpec) -> Result<spec::ConfigRef> {
        let config_ref = spec::ConfigRef {
            schema_id: binding.schema_id.clone(),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                *binding.content_digest.digest(),
            ),
            digest: binding.content_digest.clone(),
            byte_len: binding.byte_len as u64,
            media_type: spec::MediaType::new("application/json")
                .map_err(|error| lower(error.to_string()))?,
        };
        self.insert_config_ref(config_ref.clone())?;
        Ok(config_ref)
    }

    fn insert_config_ref(&mut self, config_ref: spec::ConfigRef) -> Result<()> {
        let key = config_ref_key(&config_ref);
        if let Some(existing) = self.config_refs.get(&key) {
            if existing != &config_ref {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!(
                        "conflicting config ref for schema {} digest {}",
                        config_ref.schema_id, config_ref.digest
                    ),
                ));
            }
        } else {
            self.config_refs.insert(key, config_ref);
        }
        Ok(())
    }

    fn insert_descriptor(&mut self, descriptor: spec::DescriptorIdentity) -> Result<()> {
        let key = descriptor_id(&descriptor).as_str().to_owned();
        if let Some(existing) = self.descriptor_identities.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting descriptor identity for {key}"),
                ));
            }
        } else {
            self.descriptor_identities.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_value_lineage(&mut self, lineage: spec::ValueLineage) -> Result<()> {
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if let Some(existing) = self.value_lineages.get(&key) {
            if existing != &lineage {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("conflicting value lineage for {key}"),
                ));
            }
        } else {
            self.value_lineages.insert(key, lineage);
        }
        Ok(())
    }

    fn insert_cell(&mut self, cell: spec::CellSpec) -> Result<()> {
        let key = cell.cell_id.as_str().to_owned();
        if self.cell_info.contains_key(&key) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
        self.cell_info.insert(
            key,
            CellInfo {
                producer: cell.producer.clone(),
                semantic_type_id: cell.semantic_type_id.clone(),
                schema_id: cell.schema_id.clone(),
                value_lineage: cell.value_lineage.clone(),
            },
        );
        self.cells.push(cell);
        Ok(())
    }

    fn predecessors_for_inputs(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            match self
                .cell_info
                .get(cell_id.as_str())
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("input cell {cell_id} has no producer"),
                    )
                })?
                .producer
                .clone()
            {
                spec::CellProducer::Node(node_id) => {
                    predecessors.insert(node_id);
                }
                spec::CellProducer::Seed(_) => {}
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn authoring_provenance(&self) -> Result<spec::AuthoringProvenance> {
        let frame_digests = self
            .draft
            .operation_lineage()
            .iter()
            .map(|frame| frame.lineage_digest.as_str())
            .collect::<Vec<_>>();
        let descriptor = spec::CompositionDescriptor {
            descriptor_id: descriptor_id_json(serde_json::json!({
                "kind": "typed_program_draft",
                "root_scope_id": self.draft.root_scope_id().as_str(),
                "operation_frames": frame_digests,
                "state_nodes": self.draft.state_nodes().len(),
            }))?,
            name: "typed-program-draft".to_owned(),
            version: "mfm.typed.program_draft.v1".to_owned(),
        };
        let config_hash = content_digest_json(serde_json::json!({
            "root_key": self.draft.root_key().as_str(),
            "root_scope_id": self.draft.root_scope_id().as_str(),
        }))?;
        if self.draft.operation_lineage().is_empty() {
            Ok(spec::AuthoringProvenance::StateComposition {
                descriptor,
                config_hash,
            })
        } else {
            Ok(spec::AuthoringProvenance::MixedComposition {
                descriptor,
                config_hash,
            })
        }
    }
}

fn validate_typed_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<()> {
    spec.spec_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    validate_contract_header(spec)?;
    let scope_ids = validate_scopes(&spec.scopes)?;
    let descriptor_index = DescriptorIndex::new(&spec.descriptor_identities)?;
    validate_descriptor_authority(&descriptor_index, registry)?;
    let config_refs = validate_config_refs(&spec.config_refs)?;
    let lineage_index = validate_value_lineages(&spec.value_lineages, &scope_ids)?;
    let cell_index = validate_cells(&spec.cells, &scope_ids, &lineage_index)?;
    validate_seeds(&spec.seeds, &scope_ids, &cell_index)?;
    let node_index = validate_nodes(
        &spec.nodes,
        &scope_ids,
        &descriptor_index,
        &config_refs,
        &cell_index,
        &lineage_index,
    )?;
    validate_operation_lineage(
        &spec.planning_lineage,
        &descriptor_index,
        &config_refs,
        &cell_index,
    )?;
    validate_public_outputs(
        &spec.public_outputs,
        &descriptor_index,
        &cell_index,
        &node_index,
    )?;
    validate_lineage_references(&spec.value_lineages, &cell_index, &config_refs)?;
    Ok(())
}

fn validate_contract_header(spec: &spec::TypedExecutionSpec) -> Result<()> {
    if spec.spec_version.as_str() != spec::SPEC_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec version {}", spec.spec_version),
        ));
    }
    if spec.media_type.as_str() != spec::MEDIA_TYPE {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec media type {}", spec.media_type.as_str()),
        ));
    }
    if spec.canonicalization != DigestAlgorithm::Sha256JcsV1 {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "typed specs must use sha256-jcs-v1",
        ));
    }
    if spec.lowering_version.as_str() != spec::LOWERING_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected lowering version {}", spec.lowering_version),
        ));
    }
    Ok(())
}

fn validate_scopes(scopes: &[spec::ScopeSpec]) -> Result<BTreeSet<String>> {
    if scopes.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTopology,
            "typed spec must contain at least one scope",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut parents = BTreeMap::new();
    let mut root_count = 0_usize;
    for scope in scopes {
        validate_planning_lineage(&scope.planning_lineage)?;
        if !ids.insert(scope.scope_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate scope id {}", scope.scope_id),
            ));
        }
        if scope.parent_scope_id.is_none() {
            root_count += 1;
        }
        let expected = scope_id_from_spec(
            scope.parent_scope_id.as_ref(),
            scope.stable_key.as_str(),
            &scope.planning_lineage,
        )?;
        if scope.scope_id != expected {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("scope {} is not stable-id derived", scope.scope_id),
            ));
        }
        parents.insert(
            scope.scope_id.as_str().to_owned(),
            scope
                .parent_scope_id
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
        );
    }
    if root_count != 1 {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!("typed spec must contain exactly one root scope, found {root_count}"),
        ));
    }
    for scope in scopes {
        if let Some(parent) = &scope.parent_scope_id {
            if !ids.contains(parent.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "scope {} references unknown parent {parent}",
                        scope.scope_id
                    ),
                ));
            }
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(scope.scope_id.as_str().to_owned());
        while let Some(scope_id) = current {
            if !seen.insert(scope_id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("scope {} participates in a parent cycle", scope.scope_id),
                ));
            }
            current = parents.get(&scope_id).cloned().flatten();
        }
    }
    Ok(ids)
}

#[derive(Debug)]
struct ConfigIndex {
    keys: BTreeSet<String>,
    ref_digests: BTreeSet<String>,
}

impl ConfigIndex {
    fn contains_ref(&self, config_ref: &spec::ConfigRef) -> bool {
        self.keys.contains(&config_ref_key(config_ref))
    }

    fn contains_ref_digest(&self, digest: &ContentDigest) -> bool {
        self.ref_digests.contains(digest.as_str())
    }
}

fn validate_config_refs(config_refs: &[spec::ConfigRef]) -> Result<ConfigIndex> {
    let mut keys = BTreeSet::new();
    let mut ref_digests = BTreeSet::new();
    for config in config_refs {
        if config.byte_len == 0 {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} has zero byte length", config.digest),
            ));
        }
        let expected_artifact =
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *config.digest.digest());
        if config.artifact_id != expected_artifact {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} artifact id mismatch", config.digest),
            ));
        }
        if !keys.insert(config_ref_key(config)) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "duplicate config ref schema {} digest {}",
                    config.schema_id, config.digest
                ),
            ));
        }
        ref_digests.insert(config_ref_digest(config)?.as_str().to_owned());
    }
    Ok(ConfigIndex { keys, ref_digests })
}

#[derive(Debug)]
struct DescriptorIndex<'a> {
    states: BTreeMap<String, &'a spec::StateDescriptorIdentity>,
    operations: BTreeMap<String, &'a spec::OperationDescriptorIdentity>,
    renderers: BTreeMap<String, &'a spec::RendererDescriptorIdentity>,
}

impl<'a> DescriptorIndex<'a> {
    fn new(descriptors: &'a [spec::DescriptorIdentity]) -> Result<Self> {
        let mut index = Self {
            states: BTreeMap::new(),
            operations: BTreeMap::new(),
            renderers: BTreeMap::new(),
        };
        let mut all = BTreeSet::new();
        for descriptor in descriptors {
            let id = descriptor_id(descriptor).as_str().to_owned();
            if !all.insert(id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("duplicate descriptor identity {id}"),
                ));
            }
            match descriptor {
                spec::DescriptorIdentity::State(state) => {
                    index.states.insert(id, state);
                }
                spec::DescriptorIdentity::Operation(operation) => {
                    index.operations.insert(id, operation);
                }
                spec::DescriptorIdentity::Renderer(renderer) => {
                    index.renderers.insert(id, renderer);
                }
            }
        }
        Ok(index)
    }

    fn state(&self, id: &DescriptorId) -> Result<&'a spec::StateDescriptorIdentity> {
        self.states.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing state descriptor identity {id}"),
            )
        })
    }

    fn operation(&self, id: &DescriptorId) -> Result<&'a spec::OperationDescriptorIdentity> {
        self.operations.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing operation descriptor identity {id}"),
            )
        })
    }

    fn renderer(&self, id: &DescriptorId) -> Result<&'a spec::RendererDescriptorIdentity> {
        self.renderers.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("missing renderer descriptor identity {id}"),
            )
        })
    }
}

fn validate_descriptor_authority(
    descriptors: &DescriptorIndex<'_>,
    registry: &CertificationRegistry,
) -> Result<()> {
    for descriptor in descriptors.states.values() {
        validate_state_descriptor_identity(descriptor)?;
        if validate_builtin_framework_state_descriptor(descriptor)? {
            continue;
        }
        let Some(registered) = registry.states.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.operations.values() {
        validate_operation_descriptor_identity(descriptor)?;
        let Some(registered) = registry.operations.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.renderers.values() {
        let expected = renderer_descriptor_id(descriptor)?;
        if descriptor.descriptor_id != expected {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "renderer descriptor {} is not content addressed",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_state_descriptor_identity(descriptor: &spec::StateDescriptorIdentity) -> Result<()> {
    let effect = effect_descriptor_for_kind(&descriptor.effect_kind)?;
    if descriptor.effect_class != effect.class.as_str()
        || descriptor.effect_name != effect.name
        || descriptor.effect_version != effect.version
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} effect metadata mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    let expected_runner = match effect.class {
        EffectClass::Pure => "pure",
        EffectClass::ReadExternal => "read_external",
        EffectClass::ManagedPlatformWrite => "managed_platform_write",
        EffectClass::ApplySideEffect => "apply_side_effect",
    };
    if descriptor.runner != expected_runner {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} runner mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    match (effect.class, &descriptor.side_effect_contract_digest) {
        (EffectClass::ApplySideEffect, Some(_)) => {}
        (EffectClass::ApplySideEffect, None) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "side-effect state descriptor {} is missing contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "non-side-effect state descriptor {} carries contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    let expected = state_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_operation_descriptor_identity(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<()> {
    let expected = operation_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "operation descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_builtin_framework_state_descriptor(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<bool> {
    let expected = match descriptor.name.as_str() {
        "mfm.framework.bridge_same_value" => Some(framework_bridge_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.render_public_outputs" => Some(framework_render_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        _ => None,
    };
    let Some(expected) = expected else {
        return Ok(false);
    };
    if *descriptor != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework state descriptor {} does not match built-in authority",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(true)
}

fn validate_value_lineages(
    lineages: &[spec::ValueLineage],
    scope_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, spec::ValueLineage>> {
    let mut index = BTreeMap::new();
    for lineage in lineages {
        if !scope_ids.contains(lineage.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("lineage references unknown scope {}", lineage.scope_id),
            ));
        }
        validate_planning_lineage(&lineage.planning_lineage)?;
        validate_domain_keys(&lineage.domain_keys)?;
        let expected = value_lineage_digest(lineage)?;
        if lineage.lineage_ref.lineage_digest != expected {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "value lineage {} is not content addressed",
                    lineage.lineage_ref.lineage_digest
                ),
            ));
        }
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if index.insert(key.clone(), lineage.clone()).is_some() {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("duplicate value lineage {key}"),
            ));
        }
    }
    Ok(index)
}

fn validate_cells(
    cells: &[spec::CellSpec],
    scope_ids: &BTreeSet<String>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
) -> Result<BTreeMap<String, spec::CellSpec>> {
    let mut index = BTreeMap::new();
    for cell in cells {
        if !scope_ids.contains(cell.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "cell {} references unknown scope {}",
                    cell.cell_id, cell.scope_id
                ),
            ));
        }
        let lineage = lineages
            .get(cell.value_lineage.lineage_digest.as_str())
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("cell {} references missing lineage", cell.cell_id),
                )
            })?;
        if lineage.scope_id != cell.scope_id || lineage.producer != cell.producer {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "cell {} lineage does not match producer/scope",
                    cell.cell_id
                ),
            ));
        }
        let expected_cell_id = cell_id_from_parts(
            &cell.scope_id,
            &cell.producer,
            &cell.semantic_type_id,
            &cell.schema_id,
        )?;
        if cell.cell_id != expected_cell_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("cell {} is not stable-id derived", cell.cell_id),
            ));
        }
        if index
            .insert(cell.cell_id.as_str().to_owned(), cell.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
    }
    Ok(index)
}

fn validate_seeds(
    seeds: &[spec::SeedSpec],
    scope_ids: &BTreeSet<String>,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut seed_ids = BTreeSet::new();
    for seed in seeds {
        if !seed_ids.insert(seed.seed_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate seed id {}", seed.seed_id),
            ));
        }
        let expected_seed_id = seed_id_from_spec(seed)?;
        if seed.seed_id != expected_seed_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("seed {} is not stable-id derived", seed.seed_id),
            ));
        }
        if !scope_ids.contains(seed.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references unknown scope {}",
                    seed.seed_id, seed.scope_id
                ),
            ));
        }
        let cell = cells.get(seed.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ),
            )
        })?;
        if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
            || cell.scope_id != seed.scope_id
            || cell.schema_id != seed.schema_id
            || cell.semantic_type_id != seed.semantic_type_id
        {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!("seed {} cell metadata mismatch", seed.seed_id),
            ));
        }
    }
    Ok(())
}

fn validate_nodes(
    nodes: &[spec::NodeSpec],
    scope_ids: &BTreeSet<String>,
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
) -> Result<BTreeMap<String, spec::NodeSpec>> {
    let mut index = BTreeMap::new();
    for node in nodes {
        if !scope_ids.contains(node.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "node {} references unknown scope {}",
                    node.node_id, node.scope_id
                ),
            ));
        }
        if !config_refs.contains_ref(&node.config_ref) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "node {} references missing config {}",
                    node.node_id, node.config_ref.digest
                ),
            ));
        }
        let descriptor = descriptors.state(&node.descriptor_id)?;
        let config_ref_digest = config_ref_digest(&node.config_ref)?;
        if descriptor.state_kind != node.state_kind
            || descriptor.state_version != node.state_version
            || descriptor.config_schema_id != node.config_ref.schema_id
            || descriptor.input_schema_id != node.input_bindings.input_schema_id
            || descriptor.effect_kind != node.effect_kind
            || descriptor.capabilities != node.capability_bindings
            || descriptor.side_effect_contract_digest
                != node
                    .side_effect
                    .as_ref()
                    .map(|contract| contract.contract_digest.clone())
        {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!("node {} descriptor metadata mismatch", node.node_id),
            ));
        }
        validate_effect_capabilities(&node.effect_kind, &node.capability_bindings)?;
        validate_side_effect_contract(node, descriptor)?;
        let input_cells = validate_input_binding(&node.input_bindings, cells)?;
        let expected_node_id = match &node.framework {
            Some(spec::FrameworkNodeSpec::Bridge(bridge)) => {
                bridge_node_id_from_spec(node.stable_key.as_str(), bridge)?
            }
            Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => render_node_id(
                &node.scope_id,
                node.stable_key.as_str(),
                &render.output_spec_digest,
            )?,
            None => state_node_id_from_spec(node, &config_ref_digest)?,
        };
        if node.node_id != expected_node_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("node {} is not stable-id derived", node.node_id),
            ));
        }
        let expected_predecessors = predecessor_nodes(&input_cells, cells)?;
        if expected_predecessors != node.deterministic_predecessors {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "node {} deterministic predecessors do not match input cells: expected {:?}, got {:?}",
                    node.node_id, expected_predecessors, node.deterministic_predecessors
                ),
            ));
        }
        let output = cells.get(node.output_cell.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "node {} output cell {} is missing",
                    node.node_id, node.output_cell
                ),
            )
        })?;
        if output.producer != spec::CellProducer::Node(node.node_id.clone())
            || output.scope_id != node.scope_id
            || output.schema_id != descriptor.output_schema_id
            || output.semantic_type_id != descriptor.output_semantic_type_id
        {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!("node {} output cell metadata mismatch", node.node_id),
            ));
        }
        let lineage = lineages
            .get(output.value_lineage.lineage_digest.as_str())
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("node {} output lineage is missing", node.node_id),
                )
            })?;
        if lineage.producer != spec::CellProducer::Node(node.node_id.clone()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("node {} output lineage producer mismatch", node.node_id),
            ));
        }
        let expected_lineage = expected_node_lineage(node, &input_cells, &config_ref_digest)?;
        if node.framework.is_some()
            && sorted_cell_ids(input_cells.clone())
                != sorted_cell_ids(expected_lineage.input_cells.clone())
        {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!("framework node {} input binding mismatch", node.node_id),
            ));
        }
        if lineage.config_ref_digest != expected_lineage.config_ref_digest
            || sorted_cell_ids(lineage.input_cells.clone())
                != sorted_cell_ids(expected_lineage.input_cells)
            || lineage.transform_policy != expected_lineage.transform_policy
        {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("node {} output lineage input/config mismatch", node.node_id),
            ));
        }
        if index
            .insert(node.node_id.as_str().to_owned(), node.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate node id {}", node.node_id),
            ));
        }
    }
    validate_framework_nodes(nodes, cells, descriptors)?;
    validate_node_graph_acyclic(&index)?;
    Ok(index)
}

fn validate_operation_lineage(
    frames: &[spec::OperationLineageFrameSpec],
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for frame in frames {
        if !ids.insert(frame.operation_instance_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "duplicate operation instance {}",
                    frame.operation_instance_id
                ),
            ));
        }
        validate_planning_lineage(&frame.parent_planning_lineage)?;
        let descriptor = descriptors.operation(&frame.operation_descriptor_id)?;
        if descriptor.input_schema_id != frame.input_bindings.input_schema_id {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation frame {} input schema does not match descriptor",
                    frame.operation_instance_id
                ),
            ));
        }
        if !config_refs.contains_ref_digest(&frame.config_ref_digest) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} references missing config {}",
                    frame.operation_instance_id, frame.config_ref_digest
                ),
            ));
        }
        validate_input_binding(&frame.input_bindings, cells)?;
        if frame.input_binding_digest != frame.input_bindings.digest {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} input binding digest mismatch",
                    frame.operation_instance_id
                ),
            ));
        }
        let expected_instance = operation_instance_id_from_spec(frame, descriptor)?;
        if frame.operation_instance_id != expected_instance {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "operation instance {} is not stable-id derived",
                    frame.operation_instance_id
                ),
            ));
        }
        for cell_id in &frame.output_cells {
            if !cells.contains_key(cell_id.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "operation frame {} returned missing cell {}",
                        frame.operation_instance_id, cell_id
                    ),
                ));
            }
        }
        let expected_lineage = operation_lineage_frame_digest_from_spec(frame, descriptor, cells)?;
        if frame.lineage_digest != expected_lineage {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "operation frame {} lineage is not content addressed",
                    frame.operation_instance_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_public_outputs(
    public_outputs: &spec::PublicOutputSpec,
    descriptors: &DescriptorIndex<'_>,
    cells: &BTreeMap<String, spec::CellSpec>,
    nodes: &BTreeMap<String, spec::NodeSpec>,
) -> Result<()> {
    if public_outputs.outputs.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            "public output spec must declare at least one output",
        ));
    }
    descriptors.renderer(&public_outputs.renderer_descriptor.descriptor_id)?;
    let mut fields = BTreeSet::new();
    for output in &public_outputs.outputs {
        if !fields.insert(output.public_field_path.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "duplicate public output field {}",
                    output.public_field_path.as_str()
                ),
            ));
        }
        let cell = cells.get(output.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("public output cell {} is missing", output.cell_id),
            )
        })?;
        if cell.producer != output.producer
            || cell.scope_id != output.scope_id
            || cell.semantic_type_id != output.semantic_type_id
            || cell.schema_id != output.schema_id
            || cell.value_lineage != output.value_lineage
        {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "public output {} does not match certified cell",
                    output.public_field_path.as_str()
                ),
            ));
        }
    }
    let output_digest = public_outputs
        .digest()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let render_nodes = nodes.values().filter_map(|node| match &node.framework {
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Some((node, render)),
        _ => None,
    });
    let mut matching = 0_usize;
    for (node, render) in render_nodes {
        if render.public_schema_id == public_outputs.public_schema_id
            && render.output_spec_digest == output_digest
            && render.renderer_descriptor == public_outputs.renderer_descriptor
            && render.required_cells == public_outputs.outputs
        {
            matching += 1;
            let expected_predecessors = predecessor_nodes(
                &render
                    .required_cells
                    .iter()
                    .map(|cell| cell.cell_id.clone())
                    .collect::<Vec<_>>(),
                cells,
            )?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(problem(
                    ProblemClass::InvalidTerminalShape,
                    format!(
                        "render node {} predecessors do not match public outputs",
                        node.node_id
                    ),
                ));
            }
        }
    }
    if matching != 1 {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            format!("expected exactly one public output render node, found {matching}"),
        ));
    }
    Ok(())
}

fn validate_lineage_references(
    lineages: &[spec::ValueLineage],
    cells: &BTreeMap<String, spec::CellSpec>,
    config_refs: &ConfigIndex,
) -> Result<()> {
    for lineage in lineages {
        if let Some(config_ref_digest) = &lineage.config_ref_digest {
            if !config_refs.contains_ref_digest(config_ref_digest) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing config {config_ref_digest}"),
                ));
            }
        }
        for input in &lineage.input_cells {
            if !cells.contains_key(input.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing input cell {input}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_framework_nodes(
    nodes: &[spec::NodeSpec],
    cells: &BTreeMap<String, spec::CellSpec>,
    descriptors: &DescriptorIndex<'_>,
) -> Result<()> {
    for node in nodes {
        match &node.framework {
            Some(spec::FrameworkNodeSpec::Bridge(bridge)) => {
                if bridge.target_cell_id != node.output_cell
                    || bridge.target_scope_id != node.scope_id
                    || bridge.semantic_type_id
                        != cells
                            .get(node.output_cell.as_str())
                            .map(|cell| cell.semantic_type_id.clone())
                            .ok_or_else(|| {
                                problem(
                                    ProblemClass::InvalidTopology,
                                    format!("bridge node {} missing target cell", node.node_id),
                                )
                            })?
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!("bridge node {} metadata mismatch", node.node_id),
                    ));
                }
                if !cells.contains_key(bridge.source_cell_id.as_str()) {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!("bridge node {} source cell is missing", node.node_id),
                    ));
                }
            }
            Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => {
                descriptors.renderer(&render.renderer_descriptor.descriptor_id)?;
            }
            None => {}
        }
    }
    Ok(())
}

struct ExpectedNodeLineage {
    input_cells: Vec<CellId>,
    config_ref_digest: Option<ContentDigest>,
    transform_policy: spec::LineageTransformPolicy,
}

fn expected_node_lineage(
    node: &spec::NodeSpec,
    input_cells: &[CellId],
    config_ref_digest: &ContentDigest,
) -> Result<ExpectedNodeLineage> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::Bridge(bridge)) => Ok(ExpectedNodeLineage {
            input_cells: vec![bridge.source_cell_id.clone()],
            config_ref_digest: None,
            transform_policy: spec::LineageTransformPolicy::SameValueBridge,
        }),
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Ok(ExpectedNodeLineage {
            input_cells: render
                .required_cells
                .iter()
                .map(|cell| cell.cell_id.clone())
                .collect(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        None => Ok(ExpectedNodeLineage {
            input_cells: input_cells.to_vec(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
    }
}

fn validate_node_graph_acyclic(nodes: &BTreeMap<String, spec::NodeSpec>) -> Result<()> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    fn visit(
        node_id: &str,
        nodes: &BTreeMap<String, spec::NodeSpec>,
        marks: &mut BTreeMap<String, Mark>,
    ) -> Result<()> {
        match marks.get(node_id) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("node graph contains a cycle at {node_id}"),
                ));
            }
            None => {}
        }
        let node = nodes.get(node_id).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("node graph references missing predecessor {node_id}"),
            )
        })?;
        marks.insert(node_id.to_owned(), Mark::Visiting);
        for predecessor in &node.deterministic_predecessors {
            visit(predecessor.as_str(), nodes, marks)?;
        }
        marks.insert(node_id.to_owned(), Mark::Done);
        Ok(())
    }

    let mut marks = BTreeMap::new();
    for node_id in nodes.keys() {
        visit(node_id, nodes, &mut marks)?;
    }
    Ok(())
}

fn validate_effect_capabilities(
    effect_kind: &EffectKind,
    capabilities: &CapabilitySetDescriptor,
) -> Result<()> {
    let (class, name) = effect_class_for_kind(effect_kind)?;
    capabilities
        .validate_for_effect_class(class, name)
        .map_err(|error| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("invalid capability set for effect {effect_kind}: {error}"),
            )
        })
}

fn validate_side_effect_contract(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<()> {
    let (class, _) = effect_class_for_kind(&node.effect_kind)?;
    match (class, &node.side_effect) {
        (EffectClass::ApplySideEffect, Some(contract))
            if descriptor.side_effect_contract_digest.as_ref()
                == Some(&contract.contract_digest) =>
        {
            Ok(())
        }
        (EffectClass::ApplySideEffect, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} contract digest does not match descriptor",
                node.node_id
            ),
        )),
        (EffectClass::ApplySideEffect, None) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} is missing side-effect contract",
                node.node_id
            ),
        )),
        (_, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "non-side-effect node {} carries side-effect contract",
                node.node_id
            ),
        )),
        _ => Ok(()),
    }
}

fn validate_input_binding(
    binding: &spec::InputBindingSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<CellId>> {
    let expected_digest = content_digest_json(input_node_json(&binding.root))?;
    if binding.digest != expected_digest {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "input binding {} is not content addressed",
                binding.input_descriptor_id
            ),
        ));
    }
    let mut input_cells = Vec::new();
    validate_input_node(&binding.root, cells, &mut input_cells)?;
    Ok(input_cells)
}

fn validate_input_node(
    node: &spec::InputBindingNodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    input_cells: &mut Vec<CellId>,
) -> Result<()> {
    match node {
        spec::InputBindingNodeSpec::Unit => Ok(()),
        spec::InputBindingNodeSpec::Cell(cell) => {
            let produced = cells.get(cell.cell_id.as_str()).ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("input binding references missing cell {}", cell.cell_id),
                )
            })?;
            if produced.semantic_type_id != cell.semantic_type_id
                || produced.schema_id != cell.schema_id
                || produced.value_lineage != cell.value_lineage
            {
                return Err(problem(
                    ProblemClass::InvalidInterfaceWiring,
                    format!(
                        "input binding for cell {} does not match cell metadata",
                        cell.cell_id
                    ),
                ));
            }
            input_cells.push(cell.cell_id.clone());
            Ok(())
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.field_path.as_str().to_owned()) {
                    return Err(problem(
                        ProblemClass::InvalidInterfaceWiring,
                        format!("duplicate input field {}", field.field_path.as_str()),
                    ));
                }
                validate_input_node(&field.node, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(false, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(true, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
    }
}

fn validate_collection_ordering(
    non_empty: bool,
    elements: &[spec::InputBindingNodeSpec],
    ordering: spec::OrderingEvidence,
    domain_keys: &[spec::StableDomainKeyRef],
) -> Result<()> {
    if non_empty && elements.is_empty() {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "non-empty collection binding contains no elements",
        ));
    }
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => {
            if !domain_keys.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "explicit-order vectors must not carry domain keys",
                ));
            }
        }
        spec::OrderingEvidence::StableDomainKey => {
            if domain_keys.len() != elements.len() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "domain-key ordered vectors must carry one key per element",
                ));
            }
            validate_domain_keys(domain_keys)?;
        }
    }
    Ok(())
}

fn validate_domain_keys(domain_keys: &[spec::StableDomainKeyRef]) -> Result<()> {
    let mut sorted = domain_keys.to_vec();
    sorted.sort();
    if sorted != domain_keys {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "stable domain keys are not canonical sorted",
        ));
    }
    let mut seen = BTreeSet::new();
    for key in domain_keys {
        let rendered = format!("{}:{}", key.schema_id, key.content_digest);
        if !seen.insert(rendered) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                "duplicate stable domain key in collection",
            ));
        }
    }
    Ok(())
}

fn predecessor_nodes(
    input_cells: &[CellId],
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<NodeId>> {
    let mut predecessors = BTreeSet::new();
    for cell_id in input_cells {
        let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("predecessor input cell {cell_id} is missing"),
            )
        })?;
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    Ok(predecessors.into_iter().collect())
}

fn lower_input_binding(binding: &program::InputBindingSpec) -> Result<spec::InputBindingSpec> {
    Ok(spec::InputBindingSpec {
        input_schema_id: binding.input_schema_id.clone(),
        input_descriptor_id: binding.input_descriptor_id.clone(),
        root: lower_input_node(&binding.root)?,
        digest: binding.digest.clone(),
    })
}

fn lower_operation_input_binding(
    binding: &program::OperationInputBindingSpec,
) -> Result<spec::InputBindingSpec> {
    Ok(spec::InputBindingSpec {
        input_schema_id: binding.input_schema_id.clone(),
        input_descriptor_id: binding.input_descriptor_id.clone(),
        root: lower_input_node(&binding.root)?,
        digest: binding.digest.clone(),
    })
}

fn lower_input_node(node: &program::InputBindingNode) -> Result<spec::InputBindingNodeSpec> {
    match node.as_ref() {
        program::InputBindingNodeRef::Unit => Ok(spec::InputBindingNodeSpec::Unit),
        program::InputBindingNodeRef::Cell(cell) => Ok(spec::InputBindingNodeSpec::Cell(Box::new(
            spec::InputBindingCellSpec {
                field_path: input_field_path(cell.field_path().as_str())?,
                cell_id: cell.cell_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                required_terminal: lower_required_terminal(cell.required_terminal()),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
            },
        ))),
        program::InputBindingNodeRef::Tuple(elements) => elements
            .iter()
            .map(lower_input_node)
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Tuple),
        program::InputBindingNodeRef::Struct(fields) => fields
            .iter()
            .map(|field| {
                Ok(spec::NamedInputBindingSpec {
                    field_path: input_field_path(field.field_path.as_str())?,
                    node: lower_input_node(&field.node)?,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Struct),
        program::InputBindingNodeRef::Vec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::Vec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
        program::InputBindingNodeRef::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::NonEmptyVec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
    }
}

fn single_cell_input_binding(
    source: CellInfo,
    cell_id: CellId,
    field_path: &str,
) -> Result<spec::InputBindingSpec> {
    let node = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: input_field_path(field_path)?,
        cell_id,
        semantic_type_id: source.semantic_type_id.clone(),
        schema_id: source.schema_id.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: source.value_lineage.clone(),
    }));
    let digest = content_digest_json(input_node_json(&node))?;
    Ok(spec::InputBindingSpec {
        input_schema_id: source.schema_id,
        input_descriptor_id: descriptor_id_json(serde_json::json!({
            "framework": "single_cell_input",
            "semantic_type_id": source.semantic_type_id.as_str(),
        }))?,
        root: node,
        digest,
    })
}

fn lower_required_terminal(value: program::RequiredTerminal) -> spec::RequiredTerminal {
    match value {
        program::RequiredTerminal::ProducedOnly => spec::RequiredTerminal::ProducedOnly,
        program::RequiredTerminal::MaybeSkipped => spec::RequiredTerminal::MaybeSkipped,
    }
}

fn lower_ordering_evidence(value: &program::OrderingEvidence) -> spec::OrderingEvidence {
    match value {
        program::OrderingEvidence::ExplicitAuthorOrder => {
            spec::OrderingEvidence::ExplicitAuthorOrder
        }
        program::OrderingEvidence::StableDomainKey => spec::OrderingEvidence::StableDomainKey,
    }
}

fn lower_domain_key_ref(value: &program::StableDomainKeyRef) -> spec::StableDomainKeyRef {
    spec::StableDomainKeyRef {
        schema_id: value.schema_id.clone(),
        content_digest: value.content_digest.clone(),
    }
}

fn lower_bridge_kind(value: program::BridgeKind) -> spec::BridgeKind {
    match value {
        program::BridgeKind::ImportFromParent => spec::BridgeKind::ImportFromParent,
        program::BridgeKind::ExportToParent => spec::BridgeKind::ExportToParent,
    }
}

fn bridge_kind_json(value: spec::BridgeKind) -> &'static str {
    match value {
        spec::BridgeKind::ImportFromParent => "import-from-parent",
        spec::BridgeKind::ExportToParent => "export-to-parent",
    }
}

fn lower_bridge_policy(value: program::BridgePolicy) -> spec::BridgePolicy {
    match value {
        program::BridgePolicy::SameRunSameValueV1 => spec::BridgePolicy::SameRunSameValue,
    }
}

fn bridge_policy_json(value: spec::BridgePolicy) -> &'static str {
    match value {
        spec::BridgePolicy::SameRunSameValue => "same-run-same-value-v1",
    }
}

fn state_descriptor_identity_from_program(
    node: &program::StateNodeSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = effect_descriptor_for_kind(&node.effect_kind)?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: node.state_descriptor_id.clone(),
        name: node.state_descriptor_name.clone(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        config_schema_id: node.config.schema_id.clone(),
        input_schema_id: node.input.input_schema_id.clone(),
        output_schema_id: node.output_schema_id.clone(),
        output_semantic_type_id: node.output_semantic_type_id.clone(),
        effect_kind: node.effect_kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities: node.capability_bindings.clone(),
        runner: runner_kind_name(node.runner).to_owned(),
        side_effect_contract_digest: node.side_effect_contract_digest.clone(),
    })
}

fn state_descriptor_identity_from_registered(
    descriptor: &program::StateDescriptorIdentity,
    runner: program::RunnerKind,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = descriptor.effect();
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        state_kind: descriptor.kind().clone(),
        state_version: descriptor.version().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        output_semantic_type_id: descriptor.output_semantic_type_id().clone(),
        effect_kind: effect.kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version.clone(),
        capabilities: descriptor.capabilities().clone(),
        runner: runner_kind_name(runner).to_owned(),
        side_effect_contract_digest: descriptor.side_effect_contract_digest().cloned(),
    })
}

fn operation_descriptor_identity_from_program(
    frame: &program::OperationLineageFrameSpec,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: frame.operation_descriptor_id.clone(),
        name: frame.operation_name.clone(),
        operation_kind: frame.operation_kind.clone(),
        operation_version: frame.operation_version.clone(),
        config_schema_id: frame.config.schema_id.clone(),
        input_schema_id: frame.input.input_schema_id.clone(),
        output_schema_id: frame.output_schema_id.clone(),
        expansion_abi: frame.expansion_abi.to_owned(),
    }
}

fn operation_descriptor_identity_from_registered(
    descriptor: &program::OperationDescriptorIdentity,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        operation_kind: descriptor.kind().clone(),
        operation_version: descriptor.version().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        expansion_abi: descriptor.expansion_abi().to_owned(),
    }
}

fn runner_kind_name(runner: program::RunnerKind) -> &'static str {
    match runner {
        program::RunnerKind::Pure => "pure",
        program::RunnerKind::ReadExternal => "read_external",
        program::RunnerKind::ManagedPlatformWrite => "managed_platform_write",
        program::RunnerKind::ApplySideEffect => "apply_side_effect",
    }
}

fn lower_planning_lineage(lineage: &program::OperationLineage) -> spec::PlanningLineage {
    spec::PlanningLineage {
        active_operation_instances: lineage.active_instances.clone(),
        completed_operation_frames: lineage.completed_frames.clone(),
        lineage_digest: lineage.digest.clone(),
    }
}

fn empty_planning_lineage() -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(Vec::new(), Vec::new())
}

fn final_planning_lineage(
    frames: &[program::OperationLineageFrameSpec],
) -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(
        Vec::new(),
        frames
            .iter()
            .map(|frame| frame.lineage_digest.clone())
            .collect(),
    )
}

fn planning_lineage_from_parts(
    active_operation_instances: Vec<OperationInstanceId>,
    completed_operation_frames: Vec<ContentDigest>,
) -> Result<spec::PlanningLineage> {
    let lineage_digest =
        planning_lineage_digest(&active_operation_instances, &completed_operation_frames)?;
    Ok(spec::PlanningLineage {
        active_operation_instances,
        completed_operation_frames,
        lineage_digest,
    })
}

fn validate_planning_lineage(lineage: &spec::PlanningLineage) -> Result<()> {
    let expected = planning_lineage_digest(
        &lineage.active_operation_instances,
        &lineage.completed_operation_frames,
    )?;
    if lineage.lineage_digest != expected {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!(
                "planning lineage {} is not content addressed",
                lineage.lineage_digest
            ),
        ));
    }
    Ok(())
}

fn planning_lineage_digest(
    active_operation_instances: &[OperationInstanceId],
    completed_operation_frames: &[ContentDigest],
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "active_instances": active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
    }))
}

fn scope_id_from_spec(
    parent_scope_id: Option<&ScopeId>,
    key: &str,
    operation_lineage: &spec::PlanningLineage,
) -> Result<ScopeId> {
    Ok(ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "local_scope_key": key,
            "operation_lineage": operation_lineage.lineage_digest.as_str(),
            "parent_scope_id": parent_scope_id.map(ScopeId::as_str),
        }))?,
    ))
}

fn seed_id_from_spec(seed: &spec::SeedSpec) -> Result<mfm_ids::SeedId> {
    Ok(mfm_ids::SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": seed.scope_id.as_str(),
            "seed_key": seed.seed_key.as_str(),
            "semantic_type_id": seed.semantic_type_id.as_str(),
            "schema_id": seed.schema_id.as_str(),
        }))?,
    ))
}

fn stable_author_key(value: &str) -> Result<spec::StableAuthorKey> {
    spec::StableAuthorKey::new(value).map_err(|error| lower(error.to_string()))
}

fn input_field_path(value: &str) -> Result<spec::PublicFieldPath> {
    let value = if value.is_empty() { "root" } else { value };
    spec::PublicFieldPath::new(value).map_err(|error| lower(error.to_string()))
}

fn lineage_ref(digest: &ContentDigest) -> spec::ValueLineageRef {
    spec::ValueLineageRef {
        lineage_digest: digest.clone(),
    }
}

fn value_lineage_digest(lineage: &spec::ValueLineage) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_ref_digest": lineage.config_ref_digest.as_ref().map(ContentDigest::as_str),
        "domain_keys": domain_key_refs_json(&sorted_domain_keys(lineage.domain_keys.clone())),
        "input_cells": sorted_cell_ids(lineage.input_cells.clone())
            .iter()
            .map(CellId::as_str)
            .collect::<Vec<_>>(),
        "operation_lineage": planning_lineage_json(&lineage.planning_lineage),
        "producer": cell_producer_json(&lineage.producer),
        "scope_id": lineage.scope_id.as_str(),
        "transform_policy": lineage_transform_policy_json(lineage.transform_policy),
    }))
}

fn sorted_cell_ids(mut values: Vec<CellId>) -> Vec<CellId> {
    values.sort();
    values
}

fn sorted_domain_keys(mut values: Vec<spec::StableDomainKeyRef>) -> Vec<spec::StableDomainKeyRef> {
    values.sort();
    values
}

fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

fn lineage_transform_policy_json(policy: spec::LineageTransformPolicy) -> &'static str {
    match policy {
        spec::LineageTransformPolicy::Source => "source",
        spec::LineageTransformPolicy::StateOutput => "state_output",
        spec::LineageTransformPolicy::SameValueBridge => "same_value_bridge",
    }
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

fn config_ref_digest(config_ref: &spec::ConfigRef) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "byte_len": config_ref.byte_len,
        "content_digest": config_ref.digest.as_str(),
        "schema_id": config_ref.schema_id.as_str(),
    }))
}

fn collect_input_cells(root: &spec::InputBindingNodeSpec) -> Vec<CellId> {
    let mut cells = Vec::new();
    collect_input_cells_into(root, &mut cells);
    cells
}

fn collect_input_cells_into(root: &spec::InputBindingNodeSpec, output: &mut Vec<CellId>) {
    match root {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => output.push(cell.cell_id.clone()),
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells_into(&field.node, output);
            }
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
    }
}

fn descriptor_id(descriptor: &spec::DescriptorIdentity) -> &DescriptorId {
    match descriptor {
        spec::DescriptorIdentity::State(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Operation(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Renderer(identity) => &identity.descriptor_id,
    }
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    let json = serde_json::to_string(&value).map_err(|error| canonical(error.to_string()))?;
    let canonical_json = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| canonical(error.to_string()))?;
    Ok(canonical_json.content_digest())
}

fn digest_bytes_json(value: serde_json::Value) -> Result<DigestBytes> {
    Ok(*content_digest_json(value)?.digest())
}

fn descriptor_id_json(value: serde_json::Value) -> Result<DescriptorId> {
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    ))
}

fn state_descriptor_id_from_spec(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "capabilities": capability_set_json(&descriptor.capabilities),
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "effect": {
            "class": descriptor.effect_class.as_str(),
            "kind": descriptor.effect_kind.as_str(),
            "name": descriptor.effect_name.as_str(),
            "version": descriptor.effect_version.as_str(),
        },
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.state_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "output_semantic_type_id": descriptor.output_semantic_type_id.as_str(),
        "runner": descriptor.runner.as_str(),
        "side_effect_contract_digest": descriptor.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
        "version": descriptor.state_version.as_str(),
    }))
}

fn operation_descriptor_id_from_spec(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.operation_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "version": descriptor.operation_version.as_str(),
    }))
}

fn renderer_descriptor_id(descriptor: &spec::RendererDescriptorIdentity) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "canonicalizer_identity": descriptor.canonicalizer_identity.as_str(),
        "public_schema_id": descriptor.public_schema_id.as_str(),
        "renderer_kind": descriptor.renderer_kind.as_str(),
        "renderer_version": descriptor.renderer_version.as_str(),
    }))
}

fn schema_id_json(schema_name: &str, value: serde_json::Value) -> Result<SchemaId> {
    SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    )
    .map_err(|error| lower(error.to_string()))
}

fn state_kind_json(name: &str, value: serde_json::Value) -> Result<StateKind> {
    StateKind::new(
        "mfm.framework",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    )
    .map_err(|error| lower(error.to_string()))
}

fn framework_config_ref(kind: &str, node_id: &NodeId) -> Result<spec::ConfigRef> {
    let payload = serde_json::json!({
        "framework": kind,
        "node_id": node_id.as_str(),
    });
    let bytes = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&payload).map_err(|error| canonical(error.to_string()))?,
    )
    .map_err(|error| canonical(error.to_string()))?;
    let digest = bytes.content_digest();
    Ok(spec::ConfigRef {
        schema_id: schema_id_json(
            "mfm.framework.config",
            serde_json::json!({ "framework": kind }),
        )?,
        artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")
            .map_err(|error| lower(error.to_string()))?,
    })
}

fn renderer_descriptor(public_schema_id: &SchemaId) -> Result<spec::RendererDescriptorIdentity> {
    let renderer_kind =
        spec::RendererKind::new("public-output/json").map_err(|error| lower(error.to_string()))?;
    let renderer_version = spec::RendererVersion::new("mfm.renderer.public_output_json.v1")
        .map_err(|error| lower(error.to_string()))?;
    let canonicalizer_identity = spec::CanonicalizerIdentity::new("sha256-jcs-v1")
        .map_err(|error| lower(error.to_string()))?;
    let descriptor_id = descriptor_id_json(serde_json::json!({
        "canonicalizer_identity": canonicalizer_identity.as_str(),
        "public_schema_id": public_schema_id.as_str(),
        "renderer_kind": renderer_kind.as_str(),
        "renderer_version": renderer_version.as_str(),
    }))?;
    Ok(spec::RendererDescriptorIdentity {
        descriptor_id,
        renderer_kind,
        renderer_version,
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity,
    })
}

fn framework_bridge_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "bridge_same_value",
        serde_json::json!({ "framework": "bridge_same_value" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.bridge_same_value.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.bridge_same_value",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_render_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "render_public_outputs",
        serde_json::json!({ "framework": "render_public_outputs" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.render_public_outputs",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        effect_kind: ManagedPlatformWrite::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "managed_platform_write",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

struct FrameworkStateDescriptorParts<'a> {
    name: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    config_schema_id: &'a SchemaId,
    input_schema_id: &'a SchemaId,
    output_schema_id: &'a SchemaId,
    output_semantic_type_id: &'a SemanticTypeId,
    effect_kind: EffectKind,
    runner: &'static str,
    capabilities: CapabilitySetDescriptor,
}

fn framework_state_descriptor(
    parts: FrameworkStateDescriptorParts<'_>,
) -> Result<spec::StateDescriptorIdentity> {
    let FrameworkStateDescriptorParts {
        name,
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        effect_kind,
        runner,
        capabilities,
    } = parts;
    let effect = effect_descriptor_for_kind(&effect_kind)?;
    let descriptor_id = descriptor_id_json(serde_json::json!({
        "capabilities": capabilities.capabilities.iter().map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        }).collect::<Vec<_>>(),
        "config_schema_id": config_schema_id.as_str(),
        "effect": {
            "class": effect.class.as_str(),
            "kind": effect.kind.as_str(),
            "name": effect.name,
            "version": effect.version.as_str(),
        },
        "input_schema_id": input_schema_id.as_str(),
        "kind": state_kind.as_str(),
        "name": name,
        "output_schema_id": output_schema_id.as_str(),
        "output_semantic_type_id": output_semantic_type_id.as_str(),
        "runner": runner,
        "side_effect_contract_digest": null,
        "version": state_version.as_str(),
    }))?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind,
        state_version,
        config_schema_id: config_schema_id.clone(),
        input_schema_id: input_schema_id.clone(),
        output_schema_id: output_schema_id.clone(),
        output_semantic_type_id: output_semantic_type_id.clone(),
        effect_kind,
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities,
        runner: runner.to_owned(),
        side_effect_contract_digest: None,
    })
}

fn render_node_id(
    root_scope_id: &ScopeId,
    key: &str,
    output_spec_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "framework": "public_output_render",
            "key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "output_spec_digest": output_spec_digest.as_str(),
            "scope_id": root_scope_id.as_str(),
        }))?,
    ))
}

fn state_node_id_from_spec(
    node: &spec::NodeSpec,
    config_ref_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": config_ref_digest.as_str(),
            "input_binding_digest": node.input_bindings.digest.as_str(),
            "local_node_key": node.stable_key.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": node.scope_id.as_str(),
            "state_kind": node.state_kind.as_str(),
            "state_version": node.state_version.as_str(),
        }))?,
    ))
}

fn bridge_node_id_from_spec(key: &str, bridge: &spec::BridgeNodeSpec) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "bridge_kind": bridge_kind_json(bridge.bridge_kind),
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "policy": bridge_policy_json(bridge.policy),
            "source_cell_id": bridge.source_cell_id.as_str(),
            "source_scope_id": bridge.source_scope_id.as_str(),
            "target_scope_id": bridge.target_scope_id.as_str(),
        }))?,
    ))
}

fn operation_instance_id_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<OperationInstanceId> {
    Ok(OperationInstanceId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": frame.config_ref_digest.as_str(),
            "input_binding_digest": frame.input_binding_digest.as_str(),
            "operation_descriptor_id": descriptor.descriptor_id.as_str(),
            "operation_key": frame.operation_key.as_str(),
            "operation_kind": descriptor.operation_kind.as_str(),
            "operation_version": descriptor.operation_version.as_str(),
            "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
            "parent_scope_id": frame.scope_id.as_str(),
        }))?,
    ))
}

fn operation_lineage_frame_digest_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_digest": frame.config_ref_digest.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_digest": frame.input_binding_digest.as_str(),
        "operation_descriptor_id": descriptor.descriptor_id.as_str(),
        "operation_instance_id": frame.operation_instance_id.as_str(),
        "operation_key": frame.operation_key.as_str(),
        "operation_kind": descriptor.operation_kind.as_str(),
        "operation_version": descriptor.operation_version.as_str(),
        "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
        "output_handles": frame.output_cells
            .iter()
            .map(|cell_id| {
                let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("operation frame references missing output cell {cell_id}"),
                    )
                })?;
                Ok(serde_json::json!({
                    "cell_id": cell.cell_id.as_str(),
                    "schema_id": cell.schema_id.as_str(),
                    "scope_id": cell.scope_id.as_str(),
                    "semantic_type_id": cell.semantic_type_id.as_str(),
                    "value_lineage": cell.value_lineage.lineage_digest.as_str(),
                }))
            })
            .collect::<Result<Vec<_>>>()?,
        "scope_id": frame.scope_id.as_str(),
    }))
}

fn framework_cell_id(
    scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": { "node_id": node_id.as_str(), "kind": "node" },
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

fn cell_id_from_parts(
    scope_id: &ScopeId,
    producer: &spec::CellProducer,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": cell_producer_json(producer),
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

fn render_value_lineage_ref(
    scope_id: &ScopeId,
    node_id: &NodeId,
    input_cells: &[CellId],
    planning_lineage: &spec::PlanningLineage,
    config_ref_digest: &ContentDigest,
) -> Result<spec::ValueLineageRef> {
    Ok(spec::ValueLineageRef {
        lineage_digest: content_digest_json(serde_json::json!({
            "config_ref_digest": config_ref_digest.as_str(),
            "domain_keys": [],
            "input_cells": sorted_cell_ids(input_cells.to_vec())
                .iter()
                .map(CellId::as_str)
                .collect::<Vec<_>>(),
            "operation_lineage": planning_lineage_json(planning_lineage),
            "producer": { "kind": "node", "node_id": node_id.as_str() },
            "scope_id": scope_id.as_str(),
            "transform_policy": "state_output",
        }))?,
    })
}

fn effect_descriptor_for_kind(effect_kind: &EffectKind) -> Result<mfm_effects::EffectDescriptor> {
    let pure = Pure::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &pure.kind {
        return Ok(pure);
    }
    let managed = ManagedPlatformWrite::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &managed.kind {
        return Ok(managed);
    }
    let side_effect = ApplySideEffect::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &side_effect.kind {
        return Ok(side_effect);
    }
    let read = mfm_effects::ReadExternal::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &read.kind {
        return Ok(read);
    }
    Err(problem(
        ProblemClass::InvalidSemanticTransition,
        format!("unknown effect kind {effect_kind}"),
    ))
}

fn effect_class_for_kind(effect_kind: &EffectKind) -> Result<(EffectClass, &'static str)> {
    let effect = effect_descriptor_for_kind(effect_kind)?;
    Ok((effect.class, effect.name))
}

fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

fn domain_key_refs_json(domain_keys: &[spec::StableDomainKeyRef]) -> Vec<serde_json::Value> {
    domain_keys
        .iter()
        .map(|key| {
            serde_json::json!({
                "content_digest": key.content_digest.as_str(),
                "schema_id": key.schema_id.as_str(),
            })
        })
        .collect()
}

fn capability_set_json(descriptor: &CapabilitySetDescriptor) -> Vec<serde_json::Value> {
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
        .collect()
}

fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

fn planning_lineage_json(lineage: &spec::PlanningLineage) -> serde_json::Value {
    serde_json::json!({
        "active_instances": lineage
            .active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": lineage
            .completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
        "digest": lineage.lineage_digest.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole, NoCaps};
    use mfm_ids::{CapabilityKind, CapabilityVersion, OperationKind, OperationVersion};
    use mfm_program::{
        build_root_with_registries, CanonicalSeed, IdempotencyKey, Operation, OperationKey,
        OperationRegistryBuilder, PublicOutputKey, PureState, RootBuilder, ScopeKey,
        SideEffectState, StateKey, StateRegistryBuilder, StateResult, StateSpec,
    };
    use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.certify.test",
        name = "value",
        version = "1",
        schema = "mfm.certify.test.value"
    )]
    struct TestValue {
        amount: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct TestConfig {
        multiplier: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct OperationConfig {
        multiplier: u64,
    }

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.certify.test.public_outputs")]
    struct TestPublicOutputs<'p, 's> {
        result: mfm_program::Handle<'p, 's, TestValue>,
    }

    #[derive(OperationOutput)]
    #[mfm(schema = "mfm.certify.test.operation_outputs")]
    struct TestOperationOutputs<'p, 's> {
        result: mfm_program::Handle<'p, 's, TestValue>,
    }

    struct MultiplyState {
        config: TestConfig,
    }

    impl StateSpec for MultiplyState {
        type Config = TestConfig;
        type Input = TestValue;
        type Output = TestValue;
        type Effect = Pure;
        type Caps = NoCaps;

        fn kind() -> program::Result<StateKind> {
            StateKind::new(
                "mfm.certify.test",
                "multiply",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x11),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<StateVersion> {
            StateVersion::new("mfm.certify.test.multiply.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.multiply"
        }

        fn new(config: Self::Config) -> program::Result<Self> {
            Ok(Self { config })
        }
    }

    impl PureState for MultiplyState {
        fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
            Ok(TestValue {
                amount: input.amount * self.config.multiplier,
            })
        }
    }

    struct MutationCap;

    impl CapabilitySpec for MutationCap {
        type Role = ExternalMutationAuthorityRole;

        fn kind() -> mfm_capabilities::Result<CapabilityKind> {
            CapabilityKind::new(
                "mfm.certify.test",
                "mutation",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x61),
            )
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn version() -> mfm_capabilities::Result<CapabilityVersion> {
            CapabilityVersion::new("mfm.certify.test.mutation.v1")
                .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.mutation"
        }
    }

    struct MutatingState {
        config: TestConfig,
    }

    impl StateSpec for MutatingState {
        type Config = TestConfig;
        type Input = TestValue;
        type Output = TestValue;
        type Effect = ApplySideEffect;
        type Caps = (MutationCap,);

        fn kind() -> program::Result<StateKind> {
            StateKind::new(
                "mfm.certify.test",
                "mutating",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x62),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<StateVersion> {
            StateVersion::new("mfm.certify.test.mutating.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.mutating"
        }

        fn new(config: Self::Config) -> program::Result<Self> {
            Ok(Self { config })
        }
    }

    impl SideEffectState for MutatingState {
        type Intent = TestValue;
        type IdempotencyInput = TestValue;
        type Submission = TestValue;
        type Receipt = TestValue;
        type Confirmation = TestValue;
        type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

        fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
            Ok(TestValue {
                amount: input.amount * self.config.multiplier,
            })
        }

        fn idempotency_input(
            &self,
            _input: &Self::Input,
            intent: &Self::Intent,
        ) -> StateResult<Self::IdempotencyInput> {
            Ok(intent.clone())
        }

        fn submit<'a>(
            &'a self,
            intent: &'a Self::Intent,
            _key: &'a IdempotencyKey<Self::IdempotencyInput>,
            _caps: &'a Self::Caps,
        ) -> Self::SubmitFuture<'a> {
            std::future::ready(Ok(intent.clone()))
        }

        fn output_from_confirmation(
            &self,
            _input: &Self::Input,
            _intent: &Self::Intent,
            confirmation: &Self::Confirmation,
        ) -> StateResult<Self::Output> {
            Ok(confirmation.clone())
        }
    }

    struct MultiplyOperation;

    impl Operation for MultiplyOperation {
        type Config = OperationConfig;
        type Input<'p, 's> = mfm_program::Handle<'p, 's, TestValue>;
        type Output<'p, 's> = TestOperationOutputs<'p, 's>;

        fn kind() -> program::Result<OperationKind> {
            OperationKind::new(
                "mfm.certify.test",
                "operation-multiply",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x12),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<OperationVersion> {
            OperationVersion::new("mfm.certify.test.operation_multiply.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.operation_multiply"
        }

        fn expand<'p, 's>(
            &self,
            config: Self::Config,
            input: Self::Input<'p, 's>,
            builder: &mut mfm_program::ScopeBuilder<'p, 's>,
        ) -> program::Result<Self::Output<'p, 's>> {
            let result = builder.state::<MultiplyState, _>(
                StateKey::new("multiply-state")?,
                TestConfig {
                    multiplier: config.multiplier,
                },
                input,
            )?;
            Ok(TestOperationOutputs { result })
        }
    }

    fn reference_draft() -> program::TypedProgramDraft {
        let mut states = StateRegistryBuilder::new();
        states
            .register::<MultiplyState>()
            .expect("state registration");
        let mut operations = OperationRegistryBuilder::new();
        operations
            .register::<MultiplyOperation>()
            .expect("operation registration");
        build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            operations.snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
                )?;
                let result = root.scope().call::<MultiplyOperation, _>(
                    OperationKey::new("multiply-operation")?,
                    MultiplyOperation,
                    OperationConfig { multiplier: 3 },
                    seed,
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &TestPublicOutputs {
                        result: result.result,
                    },
                )
            },
        )
        .expect("reference draft")
    }

    fn side_effect_draft() -> program::TypedProgramDraft {
        let mut states = StateRegistryBuilder::new();
        states
            .register::<MutatingState>()
            .expect("state registration");
        build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            OperationRegistryBuilder::new().snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
                )?;
                let result = root.scope().state::<MutatingState, _>(
                    StateKey::new("mutating-state")?,
                    TestConfig { multiplier: 3 },
                    seed,
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &TestPublicOutputs { result },
                )
            },
        )
        .expect("side effect draft")
    }

    #[test]
    fn certifies_reference_program_draft() {
        let draft = reference_draft();
        let expected_public_schema = draft.public_output_spec().public_schema_id().clone();
        let certified = certify_program_draft(&draft).expect("certified");
        certified.envelope.verify_hash().expect("hash verifies");
        assert_eq!(
            certified.envelope.spec.public_outputs.public_schema_id,
            expected_public_schema
        );
        assert!(certified.envelope.spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        )));
    }

    #[test]
    fn typed_certification_rejects_problem_taxonomy() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.cells.push(spec.cells[0].clone());
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let first_input = match &mut spec.nodes[0].input_bindings.root {
                    spec::InputBindingNodeSpec::Cell(cell) => cell,
                    _ => panic!("expected cell input"),
                };
                first_input.schema_id = SchemaId::new(
                    "mfm.certify.test.wrong",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x44),
                )
                .expect("schema id");
                spec.nodes[0].input_bindings.digest =
                    content_digest_json(input_node_json(&spec.nodes[0].input_bindings.root))
                        .expect("input digest");
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.nodes[0].effect_kind = ApplySideEffect::descriptor()
                    .expect("side effect descriptor")
                    .kind;
            },
        );
        assert_rejects(&registry, &base, ProblemClass::InvalidDataShape, |spec| {
            spec.config_refs.clear();
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            let seed_lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| matches!(lineage.producer, spec::CellProducer::Seed(_)))
                .expect("seed lineage");
            seed_lineage.config_ref_digest = Some(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x55),
            ));
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidTerminalShape,
            |spec| {
                spec.public_outputs.outputs.clear();
            },
        );
    }

    #[test]
    fn typed_spec_requires_registry_authority() {
        let spec = certify_program_draft(&reference_draft())
            .expect("certified")
            .envelope
            .spec;
        let error =
            certify_typed_spec(spec, &CertificationRegistry::new()).expect_err("must reject");
        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition)
        );
    }

    #[test]
    fn certification_rejects_forged_framework_descriptor_bypass() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let mut forged = spec
                    .descriptor_identities
                    .iter()
                    .find_map(|descriptor| match descriptor {
                        spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                        _ => None,
                    })
                    .expect("state descriptor");
                forged.name = "mfm.framework.forged".to_owned();
                forged.state_kind =
                    state_kind_json("forged", serde_json::json!({ "framework": "forged" }))
                        .expect("state kind");
                forged.state_version =
                    StateVersion::new("mfm.framework.state.forged.v1").expect("state version");
                forged.descriptor_id =
                    state_descriptor_id_from_spec(&forged).expect("descriptor id");
                spec.descriptor_identities
                    .push(spec::DescriptorIdentity::State(Box::new(forged)));
            },
        );
    }

    #[test]
    fn certification_rejects_operation_input_binding_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let frame = spec
                .planning_lineage
                .first_mut()
                .expect("operation lineage frame");
            let input_cell = match &mut frame.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell,
                _ => panic!("expected operation cell input"),
            };
            input_cell.cell_id =
                CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x91));
            frame.input_bindings.digest =
                content_digest_json(input_node_json(&frame.input_bindings.root))
                    .expect("input digest");
            frame.input_binding_digest = frame.input_bindings.digest.clone();
        });
    }

    #[test]
    fn certification_rejects_stable_id_and_scope_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.nodes[0].stable_key =
                spec::StableAuthorKey::new("renamed-node").expect("node key");
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.scopes[0].parent_scope_id = Some(spec.scopes[0].scope_id.clone());
        });
    }

    #[test]
    fn certification_rejects_framework_lineage_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            let render_node_id = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                    )
                })
                .expect("render node")
                .node_id
                .clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(render_node_id.clone())
                })
                .expect("render lineage");
            lineage.input_cells.clear();
        });
    }

    #[test]
    fn certification_rejects_side_effect_contract_mismatch() {
        let draft = side_effect_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .envelope
            .spec;

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let node = spec
                    .nodes
                    .iter_mut()
                    .find(|node| node.side_effect.is_some())
                    .expect("side-effect node");
                node.side_effect = Some(spec::SideEffectContractSpec {
                    contract_digest: ContentDigest::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_byte(0x63),
                    ),
                });
            },
        );
    }

    fn assert_rejects(
        registry: &CertificationRegistry,
        base: &spec::TypedExecutionSpec,
        expected: ProblemClass,
        mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
    ) {
        let mut mutated = base.clone();
        mutate(&mut mutated);
        let error = certify_typed_spec(mutated, registry).expect_err("mutation must reject");
        assert_eq!(error.problem_class(), Some(expected), "{error}");
    }

    fn digest_byte(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }

    #[test]
    fn certification_summary_keys_are_stable() {
        let keys = [
            ProblemClass::InvalidTopology.summary_key(),
            ProblemClass::InvalidInterfaceWiring.summary_key(),
            ProblemClass::InvalidSemanticTransition.summary_key(),
            ProblemClass::InvalidDataShape.summary_key(),
            ProblemClass::InvalidDataMeaning.summary_key(),
            ProblemClass::InvalidTerminalShape.summary_key(),
        ];
        assert_eq!(
            keys,
            [
                "invalid_topology_rejected",
                "invalid_interface_wiring_rejected",
                "invalid_semantic_transition_rejected",
                "invalid_data_shape_rejected",
                "invalid_data_meaning_rejected",
                "invalid_terminal_shape_rejected"
            ]
        );
    }
}
