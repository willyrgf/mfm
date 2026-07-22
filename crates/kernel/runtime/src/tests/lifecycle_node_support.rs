use super::*;

pub(super) fn runtime_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
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

pub(super) fn append_runtime_retention_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    public_output_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf1; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf2; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf3; 32]),
    );
    let config_ref = spec::framework_config_ref("project_retention_manifest", &node_id)
        .expect("retention config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == public_output_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "project_retention_manifest",
        "public_output_receipt",
        &input_cell,
    )
    .expect("input binding");
    let pure = mfm_capabilities::Pure::descriptor().expect("pure effect");
    let receipt_schema =
        spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
    let receipt_semantic =
        spec::retention_manifest_receipt_semantic_type_id().expect("retention receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "project_retention_manifest",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf9; 32]),
    )
    .expect("state kind");
    let state_version = StateVersion::new("mfm.framework.state.project_retention_manifest.v1")
        .expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.project_retention_manifest".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: pure.kind.clone(),
                effect_class: pure.class.as_str().to_owned(),
                effect_name: pure.name.to_owned(),
                effect_version: pure.version,
                capabilities: no_caps.clone(),
                runner: "pure".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xfa),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        public_output_receipt_cell
    } else {
        input_cell.cell_id.clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/project-retention-manifest")
            .expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: pure.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(
            spec::ProjectRetentionManifestNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                public_output_receipt_cell: framework_receipt,
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

pub(super) fn append_runtime_complete_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    retention_manifest_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("complete_run", &node_id).expect("complete config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "complete_run",
        "retention_manifest_receipt",
        &input_cell,
    )
    .expect("input binding");
    let pure = mfm_capabilities::Pure::descriptor().expect("pure effect");
    let receipt_schema = spec::complete_run_receipt_schema_id().expect("complete receipt schema");
    let receipt_semantic =
        spec::complete_run_receipt_semantic_type_id().expect("complete receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "complete_run",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.complete_run.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.complete_run".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: pure.kind.clone(),
                effect_class: pure.class.as_str().to_owned(),
                effect_name: pure.name.to_owned(),
                effect_version: pure.version,
                capabilities: no_caps.clone(),
                runner: "pure".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xbb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        retention_manifest_receipt_cell
    } else {
        typed
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/complete-run").expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: pure.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::CompleteRun(
            spec::CompleteRunNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                retention_manifest_receipt_cell: framework_receipt,
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

pub(super) fn append_runtime_resolve_saga_terminal_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("resolve_saga_terminal", &node_id).expect("resolve config ref");
    let input_binding = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
        .expect("input binding");
    let pure = mfm_capabilities::Pure::descriptor().expect("pure effect");
    let receipt_schema =
        spec::resolve_saga_terminal_receipt_schema_id().expect("resolve receipt schema");
    let receipt_semantic =
        spec::resolve_saga_terminal_receipt_semantic_type_id().expect("resolve receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "resolve_saga_terminal",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.resolve_saga_terminal.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.resolve_saga_terminal".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: pure.kind.clone(),
                effect_class: pure.class.as_str().to_owned(),
                effect_name: pure.name.to_owned(),
                effect_version: pure.version,
                capabilities: no_caps.clone(),
                runner: "pure".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    let scope_id = typed.scopes.first().expect("root scope").scope_id.clone();
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xcb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/resolve-saga-terminal")
            .expect("stable key"),
        scope_id,
        state_kind,
        state_version,
        descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: pure.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(
            spec::ResolveSagaTerminalNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    });
    node_id
}
