use super::*;

pub(in crate::tests::support) struct NodeSpecFixture {
    pub(super) node_id: NodeId,
    pub(super) descriptor_id: DescriptorId,
    pub(super) scope_id: ScopeId,
    pub(super) state_name: &'static str,
    pub(super) state_kind: StateKind,
    pub(super) state_version: StateVersion,
    pub(super) effect_kind: EffectKind,
    pub(super) config_ref: spec::ConfigRef,
    pub(super) input_schema: SchemaId,
    pub(super) input_cell: CellId,
    pub(super) input_lineage: spec::ValueLineageRef,
    pub(super) output_cell: CellId,
    pub(super) output_schema: SchemaId,
    pub(super) semantic: SemanticTypeId,
    pub(super) caps: CapabilitySetDescriptor,
    pub(super) predecessors: Vec<NodeId>,
    pub(super) adapter_bindings: Vec<spec::AdapterBinding>,
    pub(super) planning: spec::PlanningLineage,
}

pub(in crate::tests::support) fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
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
        context: spec::NodeContextSpec::no_context(),
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
                context: spec::InputContextSpec::no_context(),
            })),
            digest: content(0x73),
        },
        output_cell: fixture.output_cell,
        effect_kind: fixture.effect_kind,
        capability_bindings: fixture.caps,
        adapter_bindings: fixture.adapter_bindings,
        side_effect: None,
        framework: None,
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: fixture.planning,
        deterministic_predecessors: fixture.predecessors,
    }
}

pub(in crate::tests::support) fn state_descriptor(
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
        context: spec::StateContextDescriptorSpec::no_context(),
        input_context: spec::StateInputContextContractSpec::no_context(),
        output_context: spec::StateOutputContextContractSpec::no_context(),
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
        effect_class: if runner == "read" {
            "read_external".to_owned()
        } else {
            runner.to_owned()
        },
        effect_name: if runner == "read" {
            "read_external".to_owned()
        } else {
            runner.to_owned()
        },
        effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
        capabilities,
        runner: runner.to_owned(),
        emitted_fact_descriptors: Vec::new(),
        effect_contract_digest: None,
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

pub(in crate::tests::support) fn input_node_json(
    node: &spec::InputBindingNodeSpec,
) -> serde_json::Value {
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
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

pub(in crate::tests::support) fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

pub(in crate::tests::support) fn stable_domain_key_ref_json(
    key: &spec::StableDomainKeyRef,
) -> serde_json::Value {
    serde_json::json!({
        "content_digest": key.content_digest.as_str(),
        "schema_id": key.schema_id.as_str(),
    })
}
