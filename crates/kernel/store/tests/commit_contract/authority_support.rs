use super::*;

pub(super) fn side_effect_pair_id() -> SideEffectPairId {
    side_effect_pair_id_for_contract(&default_side_effect_contract())
}

pub(super) fn side_effect_pair_id_for_contract(
    contract: &spec::SideEffectContractSpec,
) -> SideEffectPairId {
    spec::side_effect_pair_id(&submit_node_id(), &side_effect_output_cell(), contract)
        .expect("certified side-effect pair id")
}

pub(super) fn remediation_pair_id() -> SideEffectPairId {
    spec::side_effect_pair_id(
        &remediation_submit_node_id(),
        &remediation_output_cell(),
        &default_side_effect_contract(),
    )
    .expect("certified remediation side-effect pair id")
}

pub(super) fn submit_node_id() -> NodeId {
    node_id(70)
}

pub(super) fn submit_attempt_id() -> AttemptId {
    attempt_id(72)
}

pub(super) fn verify_node_id() -> NodeId {
    node_id(170)
}

pub(super) fn verify_attempt_id() -> AttemptId {
    attempt_id(172)
}

pub(super) fn remediation_submit_node_id() -> NodeId {
    node_id(180)
}

pub(super) fn remediation_submit_attempt_id() -> AttemptId {
    attempt_id(182)
}

pub(super) fn remediation_verify_node_id() -> NodeId {
    node_id(190)
}

pub(super) fn remediation_verify_attempt_id() -> AttemptId {
    attempt_id(192)
}

pub(super) fn side_effect_output_cell() -> CellId {
    cell_id(78)
}

pub(super) fn remediation_output_cell() -> CellId {
    cell_id(178)
}

pub(super) fn default_side_effect_contract() -> spec::SideEffectContractSpec {
    side_effect_contract_with_verification(spec::SideEffectVerificationSpec::Finalized { depth: 1 })
}

pub(super) fn side_effect_contract_with_verification(
    verification: spec::SideEffectVerificationSpec,
) -> spec::SideEffectContractSpec {
    spec::SideEffectContractSpec {
        contract_digest: content_digest(77),
        resource_claim: spec::ResourceClaimSpec::ManualOnly,
        verification,
    }
}

pub(super) fn saga_authority_spec(policy: SagaPolicySpec) -> spec::TypedExecutionSpec {
    saga_authority_spec_with_verification(
        policy,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(super) fn saga_authority_spec_with_verification(
    policy: SagaPolicySpec,
    verification: spec::SideEffectVerificationSpec,
) -> spec::TypedExecutionSpec {
    let planning_lineage = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content_digest(85),
    };
    let public_schema_id = schema_id("mfm.test.public_output", 3);
    let contract = side_effect_contract_with_verification(verification);
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
    let remediation_submit_node_id = remediation_submit_node_id();
    let remediation_output_cell = remediation_output_cell();
    let remediation_pair_id = spec::side_effect_pair_id(
        &remediation_submit_node_id,
        &remediation_output_cell,
        &contract,
    )
    .expect("remediation side-effect pair id");
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
                submit_node_id,
            },
        )),
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: vec![node_id(70)],
    };
    let remediation_submit_node = spec::NodeSpec {
        node_id: remediation_submit_node_id.clone(),
        stable_key: spec::StableAuthorKey::new("side-effect-remediation").expect("stable key"),
        scope_id: scope_id(181),
        state_kind: state_kind.clone(),
        state_version: state_version.clone(),
        descriptor_id: state_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings: input_bindings.clone(),
        output_cell: remediation_output_cell.clone(),
        effect_kind: effect_kind.clone(),
        capability_bindings: capability_bindings.clone(),
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: Some(contract.clone()),
        framework: None,
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    };
    let remediation_verify_node = spec::NodeSpec {
        node_id: remediation_verify_node_id(),
        stable_key: spec::StableAuthorKey::new("side-effect-remediation-verify")
            .expect("stable key"),
        scope_id: scope_id(191),
        state_kind: state_kind.clone(),
        state_version: state_version.clone(),
        descriptor_id: state_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings: input_bindings.clone(),
        output_cell: remediation_output_cell,
        effect_kind: effect_kind.clone(),
        capability_bindings: capability_bindings.clone(),
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::SideEffectVerify(
            spec::SideEffectVerifyNodeSpec {
                pair_id: remediation_pair_id,
                submit_node_id: remediation_submit_node_id.clone(),
            },
        )),
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: vec![remediation_submit_node_id.clone()],
    };
    let remediations = BTreeMap::from([(remediation_submit_node_id, remediation_submit_node)]);
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
                emitted_fact_descriptors: Vec::new(),
                runner: "mfm.test.runner".to_owned(),
                side_effect_contract_digest: Some(contract.contract_digest.clone()),
            })),
            spec::DescriptorIdentity::Renderer(Box::new(renderer_descriptor.clone())),
        ],
        config_refs: vec![config_ref.clone()],
        nodes: vec![submit_node, verify_node, remediation_verify_node],
        remediations,
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

pub(super) fn saga_authority_spec_with_authoring_config_hash(
    policy: SagaPolicySpec,
    config_hash: ContentDigest,
) -> spec::TypedExecutionSpec {
    let mut spec = saga_authority_spec(policy);
    match &mut spec.authoring {
        spec::AuthoringProvenance::StateComposition {
            config_hash: authoring_config_hash,
            ..
        }
        | spec::AuthoringProvenance::OperationExpansion {
            config_hash: authoring_config_hash,
            ..
        }
        | spec::AuthoringProvenance::MixedComposition {
            config_hash: authoring_config_hash,
            ..
        } => *authoring_config_hash = config_hash,
    }
    spec
}

pub(super) fn run_id(byte: u8) -> RunId {
    run_id_with_saga_policy(byte, &SagaPolicySpec::NoSideEffects)
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

pub(super) fn execution_claim_scope(byte: u8) -> ExecutionClaimScope {
    ExecutionClaimScope::from_run_identity_material(&run_identity_material_for_saga_policy(
        byte,
        &SagaPolicySpec::NoSideEffects,
    ))
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

pub(super) fn run_identity_material_for_spec_run_id(
    run_id: &RunId,
    certified_spec_hash: &SpecHash,
) -> events::RunIdentityMaterialV1 {
    (0..=u8::MAX)
        .map(|byte| {
            run_identity_material_for_test(certified_spec_hash.clone(), &store_scope_hex(byte))
        })
        .find(|material| {
            material
                .derive_run_id()
                .map(|derived| &derived == run_id)
                .unwrap_or(false)
        })
        .expect("test run id must be derived from certified spec identity material")
}

pub(super) fn fact_run_id(byte: u8) -> RunId {
    fact_run_id_for_node(byte, node_id(90))
}

pub(super) fn fact_run_id_for_node(byte: u8, fact_node_id: NodeId) -> RunId {
    let certified_spec_hash = fact_authority_spec_with_node(fact_node_id)
        .spec_hash()
        .expect("fact authority spec hash");
    run_identity_material_for_test(certified_spec_hash, &store_scope_hex(byte))
        .derive_run_id()
        .expect("fact test run id")
}

pub(super) fn store_scope_hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(16)
}

pub(super) fn side_effect_pair_role(
    role: events::SideEffectPairRole,
) -> (SideEffectPairId, events::SideEffectPairRole) {
    (side_effect_pair_id(), role)
}

pub(super) fn run_admitted(run_id: RunId) -> KernelEventPayload {
    run_admitted_with_saga_policy(run_id, &SagaPolicySpec::NoSideEffects)
}

pub(super) fn run_admitted_with_saga_policy(
    run_id: RunId,
    saga_policy: &SagaPolicySpec,
) -> KernelEventPayload {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let identity_material = run_identity_material_for_run_id(&run_id, saga_policy);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: saga_policy
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}

pub(super) fn fact_authority_spec_with_node(fact_node_id: NodeId) -> spec::TypedExecutionSpec {
    let mut spec = saga_authority_spec(SagaPolicySpec::NoSideEffects);
    let template = spec.nodes[0].clone();
    let fact_node = spec::NodeSpec {
        node_id: fact_node_id,
        stable_key: spec::StableAuthorKey::new("fact-node").expect("stable key"),
        scope_id: scope_id(90),
        state_kind: state_kind(90),
        state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
        descriptor_id: template.descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref,
        input_bindings: template.input_bindings,
        output_cell: cell_id(90),
        effect_kind: template.effect_kind,
        capability_bindings: template.capability_bindings,
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: vec![spec::FactDescriptorRef {
            descriptor_hash: fact_descriptor_hash(),
        }],
        side_effect: None,
        framework: None,
        planning_lineage: template.planning_lineage,
        deterministic_predecessors: Vec::new(),
    };
    spec.nodes.push(fact_node);
    spec
}

pub(super) fn run_admitted_with_fact_descriptor_for_node(
    run_id: RunId,
    fact_node_id: NodeId,
) -> KernelEventPayload {
    let authority_spec = fact_authority_spec_with_node(fact_node_id);
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("fact authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let descriptor_artifact = fact_descriptor_artifact_ref();
    let identity_material = run_identity_material_for_spec_run_id(&run_id, &certified_spec_hash);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: vec![run_artifact_ref(&descriptor_artifact)],
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: SagaPolicySpec::NoSideEffects
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}
