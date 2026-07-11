use super::*;

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
    pub(super) fn json(&self) -> serde_json::Value {
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
    /// Certified context constraint for this input cell.
    pub context: InputContextSpec,
}

impl InputBindingCellSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "cell_id": self.cell_id.as_str(),
            "context": self.context.json(),
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
    pub(super) fn as_str(self) -> &'static str {
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
    /// Certified transition-context constraint.
    pub context: CellContextSpec,
}

impl CellSpec {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "cell_id": self.cell_id.as_str(),
            "context": self.context.json(),
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
    pub(super) fn json(&self) -> serde_json::Value {
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
    pub(super) fn json(&self) -> serde_json::Value {
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
    pub(super) fn json(&self, descriptors: &DescriptorJsonIndex) -> Result<serde_json::Value> {
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

    pub(super) fn json(&self) -> Result<serde_json::Value> {
        let renderer_ref = DescriptorIdentity::Renderer(Box::new(self.renderer_descriptor.clone()))
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
    pub(super) fn json(&self) -> serde_json::Value {
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
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProducedOnly => "produced_only",
            Self::MaybeSkipped => "maybe_skipped",
        }
    }
}
