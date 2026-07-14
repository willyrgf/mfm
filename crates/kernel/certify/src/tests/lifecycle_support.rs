use super::*;

pub(super) fn lifecycle_render_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("render node")
        .output_cell
        .clone()
}

pub(super) fn lifecycle_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        })
        .expect("retention node")
        .output_cell
        .clone()
}

pub(super) trait LifecycleVariant {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool;
}

impl LifecycleVariant for spec::ProjectRetentionManifestNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(
            framework,
            spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
        )
    }
}

impl LifecycleVariant for spec::CompleteRunNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::CompleteRun(_))
    }
}

impl LifecycleVariant for spec::ResolveSagaTerminalNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    }
}

impl LifecycleVariant for spec::PublicOutputRenderNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::PublicOutputRender(_))
    }
}

pub(super) fn find_lifecycle_node_mut<T: LifecycleVariant>(
    typed: &mut spec::TypedExecutionSpec,
) -> &mut spec::NodeSpec {
    typed
        .nodes
        .iter_mut()
        .find(|node| node.framework.as_ref().is_some_and(T::matches))
        .expect("lifecycle node")
}

pub(super) fn clear_lifecycle_framework_metadata<T: LifecycleVariant>(
    typed: &mut spec::TypedExecutionSpec,
) {
    find_lifecycle_node_mut::<T>(typed).framework = None;
}

pub(super) fn append_retention_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    public_output_receipt_cell: CellId,
) -> CellId {
    let scope_id = typed.scopes[0].scope_id.clone();
    let stable_key = spec::StableAuthorKey::new("framework/project-retention-manifest")
        .expect("retention stable key");
    let retention = spec::ProjectRetentionManifestNodeSpec {
        public_schema_id: typed.public_outputs.public_schema_id.clone(),
        public_output_receipt_cell: public_output_receipt_cell.clone(),
    };
    let node_id =
        project_retention_manifest_node_id_from_spec(&scope_id, stable_key.as_str(), &retention)
            .expect("retention node id");
    let receipt_schema =
        spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
    let receipt_semantic =
        spec::retention_manifest_receipt_semantic_type_id().expect("retention receipt semantic");
    let output_cell = framework_cell_id(&scope_id, &node_id, &receipt_semantic, &receipt_schema)
        .expect("retention output cell");
    let config_ref =
        framework_config_ref("project_retention_manifest", &node_id).expect("config ref");
    let config_digest = config_ref_digest(&config_ref).expect("config digest");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == public_output_receipt_cell)
        .expect("public-output receipt cell");
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "project_retention_manifest",
        "public_output_receipt",
        input_cell,
    )
    .expect("input binding");
    let descriptor = framework_project_retention_manifest_descriptor(
        &receipt_schema,
        &receipt_semantic,
        &config_ref.schema_id,
        &input_binding.input_schema_id,
    )
    .expect("retention descriptor");
    let planning_lineage = typed.scopes[0].planning_lineage.clone();
    let lineage = render_value_lineage_ref(
        &scope_id,
        &node_id,
        std::slice::from_ref(&public_output_receipt_cell),
        &planning_lineage,
        &config_digest,
    )
    .expect("retention lineage");

    push_lifecycle_node(LifecycleNodeParts {
        typed,
        node_id,
        stable_key,
        scope_id,
        descriptor,
        config_ref,
        input_binding,
        output_cell: output_cell.clone(),
        receipt_schema,
        receipt_semantic,
        framework: spec::FrameworkNodeSpec::ProjectRetentionManifest(retention),
        planning_lineage,
        lineage,
        input_cells: vec![public_output_receipt_cell],
    });
    output_cell
}

struct LifecycleNodeParts<'a> {
    typed: &'a mut spec::TypedExecutionSpec,
    node_id: NodeId,
    stable_key: spec::StableAuthorKey,
    scope_id: ScopeId,
    descriptor: spec::StateDescriptorIdentity,
    config_ref: spec::ConfigRef,
    input_binding: spec::InputBindingSpec,
    output_cell: CellId,
    receipt_schema: SchemaId,
    receipt_semantic: SemanticTypeId,
    framework: spec::FrameworkNodeSpec,
    planning_lineage: spec::PlanningLineage,
    lineage: spec::ValueLineageRef,
    input_cells: Vec<CellId>,
}

fn push_lifecycle_node(parts: LifecycleNodeParts<'_>) {
    let LifecycleNodeParts {
        typed,
        node_id,
        stable_key,
        scope_id,
        descriptor,
        config_ref,
        input_binding,
        output_cell,
        receipt_schema,
        receipt_semantic,
        framework,
        planning_lineage,
        lineage,
        input_cells,
    } = parts;
    let predecessors = predecessors_for_test_inputs(typed, &input_cells);
    typed.config_refs.push(config_ref.clone());
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )));
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: scope_id.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        input_cells,
        config_ref_digest: Some(config_ref_digest(&config_ref).expect("config digest")),
        planning_lineage: planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    typed.nodes.push(spec::NodeSpec {
        node_id,
        stable_key,
        scope_id,
        state_kind: descriptor.state_kind,
        state_version: descriptor.state_version,
        descriptor_id: descriptor.descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: descriptor.effect_kind,
        capability_bindings: descriptor.capabilities,
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: Some(framework),
        planning_lineage,
        deterministic_predecessors: predecessors,
    });
}

pub(super) fn predecessors_for_test_inputs(
    typed: &spec::TypedExecutionSpec,
    input_cells: &[CellId],
) -> Vec<NodeId> {
    let mut predecessors = BTreeSet::new();
    for input_cell in input_cells {
        let cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == *input_cell)
            .expect("input cell");
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    predecessors.into_iter().collect()
}

pub(super) fn append_user_receipt_consumer(
    typed: &mut spec::TypedExecutionSpec,
    receipt_cell: CellId,
    stable_key: &str,
) -> NodeId {
    let template = typed
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("user node template")
        .clone();
    let descriptor = typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == template.descriptor_id =>
            {
                Some(state.as_ref().clone())
            }
            _ => None,
        })
        .expect("template descriptor");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == receipt_cell)
        .expect("receipt cell")
        .clone();
    let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: spec::PublicFieldPath::new("receipt").expect("field path"),
        cell_id: receipt_cell.clone(),
        semantic_type_id: input_cell.semantic_type_id.clone(),
        schema_id: input_cell.schema_id.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: input_cell.value_lineage.clone(),
        context: spec::InputContextSpec::no_context(),
    }));
    let input_binding = spec::InputBindingSpec {
        input_schema_id: descriptor.input_schema_id.clone(),
        input_descriptor_id: descriptor_id_json(serde_json::json!({
            "input": "framework_receipt_consumer",
            "receipt_cell": receipt_cell.as_str(),
            "stable_key": stable_key,
        }))
        .expect("input descriptor"),
        digest: content_digest_json(input_node_json(&root)).expect("input digest"),
        root,
    };
    let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
    let mut node = spec::NodeSpec {
        node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf0)),
        stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: descriptor.state_kind.clone(),
        state_version: descriptor.state_version.clone(),
        descriptor_id: descriptor.descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: input_binding,
        output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf1)),
        effect_kind: descriptor.effect_kind.clone(),
        capability_bindings: descriptor.capabilities.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: predecessors_for_test_inputs(
            typed,
            std::slice::from_ref(&receipt_cell),
        ),
    };
    node.node_id = state_node_id_from_spec(&node, &config_digest).expect("consumer node id");
    node.output_cell = cell_id_from_parts(
        &node.scope_id,
        &spec::CellProducer::Node(node.node_id.clone()),
        &descriptor.output_semantic_type_id,
        &descriptor.output_schema_id,
    )
    .expect("consumer output cell");
    let lineage = render_value_lineage_ref(
        &node.scope_id,
        &node.node_id,
        std::slice::from_ref(&receipt_cell),
        &node.planning_lineage,
        &config_digest,
    )
    .expect("consumer lineage");
    typed.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: descriptor.output_semantic_type_id,
        schema_id: descriptor.output_schema_id,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells: vec![receipt_cell],
        config_ref_digest: Some(config_digest),
        planning_lineage: node.planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    let node_id = node.node_id.clone();
    typed.nodes.push(node);
    node_id
}

pub(super) fn append_independent_user_node(
    typed: &mut spec::TypedExecutionSpec,
    stable_key: &str,
) -> NodeId {
    let template = typed
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("user node template")
        .clone();
    let descriptor = typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == template.descriptor_id =>
            {
                Some(state.as_ref().clone())
            }
            _ => None,
        })
        .expect("template descriptor");
    let input_cells = collect_input_cells(&template.input_bindings.root);
    let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
    let mut node = spec::NodeSpec {
        node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe0)),
        stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: descriptor.state_kind.clone(),
        state_version: descriptor.state_version.clone(),
        descriptor_id: descriptor.descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: template.input_bindings.clone(),
        output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe1)),
        effect_kind: descriptor.effect_kind.clone(),
        capability_bindings: descriptor.capabilities.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: template.fact_descriptor_allowlist.clone(),
        side_effect: template.side_effect.clone(),
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: predecessors_for_test_inputs(typed, &input_cells),
    };
    node.node_id = state_node_id_from_spec(&node, &config_digest).expect("node id");
    node.output_cell = cell_id_from_parts(
        &node.scope_id,
        &spec::CellProducer::Node(node.node_id.clone()),
        &descriptor.output_semantic_type_id,
        &descriptor.output_schema_id,
    )
    .expect("output cell");
    let lineage = render_value_lineage_ref(
        &node.scope_id,
        &node.node_id,
        &input_cells,
        &node.planning_lineage,
        &config_digest,
    )
    .expect("lineage");
    typed.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: descriptor.output_semantic_type_id,
        schema_id: descriptor.output_schema_id,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells,
        config_ref_digest: Some(config_digest),
        planning_lineage: node.planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    let node_id = node.node_id.clone();
    typed.nodes.push(node);
    node_id
}
