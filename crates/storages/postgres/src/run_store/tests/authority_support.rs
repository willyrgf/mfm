use super::*;

pub(super) fn saga_authority_spec(policy: SagaPolicySpec) -> spec::TypedExecutionSpec {
    let planning_lineage = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content_digest(85),
    };
    let public_schema_id = schema_id("mfm.test.public_output", 3);
    let contract = default_side_effect_contract();
    let config_ref = spec::ConfigRef {
        schema_id: schema_id("mfm.test.side_effect_config", 89),
        artifact_id: artifact_id(90),
        digest: content_digest(91),
        byte_len: 2,
        media_type: media_type("application/json"),
    };
    let input_bindings = spec::InputBindingSpec {
        input_schema_id: schema_id("mfm.test.side_effect_input", 92),
        input_descriptor_id: descriptor_id(93),
        root: spec::InputBindingNodeSpec::Unit,
        digest: content_digest(94),
    };
    let state_kind = state_kind(70);
    let state_version = StateVersion::new("mfm.test.side_effect_state.v1").expect("state version");
    let state_descriptor_id = descriptor_id(88);
    let effect_kind = EffectKind::new(
        "mfm.test",
        "side_effect",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(95),
    )
    .expect("effect kind");
    let capability_bindings =
        mfm_capabilities::CapabilitySetDescriptor::new(Vec::new()).expect("empty capabilities");
    let renderer_descriptor = spec::RendererDescriptorIdentity {
        descriptor_id: descriptor_id(96),
        renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer kind"),
        renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
            .expect("renderer version"),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
    };
    let submit_node_id = submit_node_id();
    let submit_output_cell = side_effect_output_cell();
    let pair_id = spec::side_effect_pair_id(&submit_node_id, &submit_output_cell, &contract)
        .expect("side-effect pair id");
    let submit_node = spec::NodeSpec {
        node_id: submit_node_id.clone(),
        stable_key: spec::StableAuthorKey::new("side-effect").expect("stable key"),
        scope_id: scope_id(71),
        state_kind: state_kind.clone(),
        state_version: state_version.clone(),
        descriptor_id: state_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings: input_bindings.clone(),
        output_cell: submit_output_cell.clone(),
        effect_kind: effect_kind.clone(),
        capability_bindings: capability_bindings.clone(),
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: Some(contract.clone()),
        framework: None,
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    };
    let verify_node = spec::NodeSpec {
        node_id: verify_node_id(),
        stable_key: spec::StableAuthorKey::new("side-effect-verify").expect("stable key"),
        scope_id: scope_id(171),
        state_kind: state_kind.clone(),
        state_version: state_version.clone(),
        descriptor_id: state_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings: input_bindings.clone(),
        output_cell: submit_output_cell,
        effect_kind: effect_kind.clone(),
        capability_bindings: capability_bindings.clone(),
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::SideEffectVerify(
            spec::SideEffectVerifyNodeSpec {
                pair_id,
                submit_node_id: submit_node_id.clone(),
            },
        )),
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: vec![submit_node_id],
    };
    spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: descriptor_id(86),
                name: "mfm.test.saga_authority".to_owned(),
                version: "mfm.test.saga_authority.v1".to_owned(),
            },
            config_hash: content_digest(87),
        },
        saga: policy,
        contexts: Vec::new(),
        scopes: Vec::new(),
        seeds: Vec::new(),
        descriptor_identities: vec![
            spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                descriptor_id: state_descriptor_id.clone(),
                name: "mfm.test.side_effect".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_bindings.input_schema_id.clone(),
                output_schema_id: schema_id("mfm.test.side_effect_output", 97),
                output_semantic_type_id: semantic_id("side_effect_output", 98),
                effect_kind: effect_kind.clone(),
                effect_class: "side_effect".to_owned(),
                effect_name: "side_effect".to_owned(),
                effect_version: EffectVersion::new("mfm.test.side_effect.v1")
                    .expect("effect version"),
                capabilities: capability_bindings.clone(),
                runner: "mfm.test.runner".to_owned(),
                effect_contract_digest: Some(contract.contract_digest.clone()),
                emitted_fact_descriptors: Vec::new(),
            })),
            spec::DescriptorIdentity::Renderer(Box::new(renderer_descriptor.clone())),
        ],
        config_refs: vec![config_ref.clone()],
        nodes: vec![submit_node, verify_node],
        remediations: BTreeMap::new(),
        cells: Vec::new(),
        value_lineages: Vec::new(),
        planning_lineage: Vec::new(),
        public_outputs: spec::PublicOutputSpec {
            public_schema_id: public_schema_id.clone(),
            outputs: Vec::new(),
            renderer_descriptor,
        },
    })
    .expect("saga authority spec")
}

pub(super) fn fact_authority_spec() -> spec::TypedExecutionSpec {
    let mut authority = saga_authority_spec(SagaPolicySpec::NoSideEffects);
    let planning_lineage = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content_digest(185),
    };
    let config_ref = spec::ConfigRef {
        schema_id: schema_id("mfm.test.fact_config", 134),
        artifact_id: artifact_id(135),
        digest: content_digest(136),
        byte_len: 2,
        media_type: media_type("application/json"),
    };
    let input_bindings = spec::InputBindingSpec {
        input_schema_id: schema_id("mfm.test.fact_input", 137),
        input_descriptor_id: descriptor_id(138),
        root: spec::InputBindingNodeSpec::Unit,
        digest: content_digest(139),
    };
    let fact_state_kind = state_kind(30);
    let fact_state_version = StateVersion::new("mfm.test.fact_state.v1").expect("state version");
    let fact_descriptor_id = descriptor_id(140);
    let fact_effect_kind = EffectKind::new(
        "mfm.test",
        "fact",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(141),
    )
    .expect("effect kind");
    let capability_bindings =
        mfm_capabilities::CapabilitySetDescriptor::new(Vec::new()).expect("empty capabilities");
    let fact_descriptor_ref = spec::FactDescriptorRef {
        descriptor_hash: fact_descriptor_hash(),
    };

    authority
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: fact_descriptor_id.clone(),
                name: "mfm.test.fact".to_owned(),
                state_kind: fact_state_kind.clone(),
                state_version: fact_state_version.clone(),
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_bindings.input_schema_id.clone(),
                output_schema_id: schema_id("mfm.test.fact_output", 142),
                output_semantic_type_id: semantic_id("fact_output", 143),
                effect_kind: fact_effect_kind.clone(),
                effect_class: "fact".to_owned(),
                effect_name: "fact".to_owned(),
                effect_version: EffectVersion::new("mfm.test.fact.v1").expect("effect version"),
                capabilities: capability_bindings.clone(),
                emitted_fact_descriptors: vec![fact_descriptor_ref.clone()],
                runner: "mfm.test.runner".to_owned(),
                effect_contract_digest: None,
            },
        )));
    authority.config_refs.push(config_ref.clone());
    authority.nodes.push(spec::NodeSpec {
        node_id: node_id(30),
        stable_key: spec::StableAuthorKey::new("fact").expect("stable key"),
        scope_id: scope_id(144),
        state_kind: fact_state_kind,
        state_version: fact_state_version,
        descriptor_id: fact_descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings,
        output_cell: cell_id(145),
        effect_kind: fact_effect_kind,
        capability_bindings,
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: vec![fact_descriptor_ref],
        side_effect: None,
        framework: None,
        planning_lineage,
        deterministic_predecessors: Vec::new(),
    });
    authority
}

pub(super) fn run_id(byte: u8) -> RunId {
    run_id_with_saga_policy(byte, &SagaPolicySpec::NoSideEffects)
}

pub(super) fn fact_run_id(byte: u8) -> RunId {
    run_identity_material_for_fact_spec(byte)
        .derive_run_id()
        .expect("test fact run id")
}

pub(super) fn run_id_with_saga_policy(byte: u8, policy: &SagaPolicySpec) -> RunId {
    run_identity_material_for_saga_policy(byte, policy)
        .derive_run_id()
        .expect("test run id")
}

pub(super) fn run_identity_material_for_saga_policy(
    byte: u8,
    policy: &SagaPolicySpec,
) -> events::RunIdentityMaterialV1 {
    let authority_spec = saga_authority_spec(policy.clone());
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    run_identity_material_for_test(certified_spec_hash, &store_scope_hex(byte))
}

pub(super) fn run_identity_material_for_fact_spec(byte: u8) -> events::RunIdentityMaterialV1 {
    let certified_spec_hash = fact_authority_spec()
        .spec_hash()
        .expect("fact authority spec hash");
    run_identity_material_for_test(certified_spec_hash, &store_scope_hex(byte))
}

pub(super) fn run_identity_material_for_run_id(
    run_id: &RunId,
    policy: &SagaPolicySpec,
) -> events::RunIdentityMaterialV1 {
    (0..=u8::MAX)
        .map(|byte| run_identity_material_for_saga_policy(byte, policy))
        .find(|material| {
            material
                .derive_run_id()
                .map(|derived| &derived == run_id)
                .unwrap_or(false)
        })
        .expect("test run id must be derived from saga policy identity material")
}

pub(super) fn run_identity_material_for_fact_run_id(
    run_id: &RunId,
) -> events::RunIdentityMaterialV1 {
    (0..=u8::MAX)
        .map(run_identity_material_for_fact_spec)
        .find(|material| {
            material
                .derive_run_id()
                .map(|derived| &derived == run_id)
                .unwrap_or(false)
        })
        .expect("test run id must be derived from fact identity material")
}

pub(super) fn store_scope_hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(16)
}
