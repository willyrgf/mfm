use super::*;

const SPEC_MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";

macro_rules! typed_commit_request {
    (
        run_id: $run_id:expr,
        expected_next_seq: $expected_next_seq:expr,
        commit_key: $commit_key:expr,
        payloads: $payloads:expr,
        required_artifacts: $required_artifacts:expr,
        preconditions: $preconditions:expr $(,)?
    ) => {
        CommitRequest::from_payloads(
            $run_id,
            $expected_next_seq,
            $commit_key,
            $payloads,
            $required_artifacts,
            $preconditions,
        )
        .expect("typed commit request")
    };
}

#[path = "admission.rs"]
mod admission_tests;
#[path = "codec.rs"]
mod codec_tests;
#[path = "commit.rs"]
mod commit_tests;
#[path = "fact_retention_support.rs"]
mod fact_retention_support;
#[path = "facts_retention.rs"]
mod facts_retention_tests;
#[path = "lifecycle.rs"]
mod lifecycle_tests;
#[path = "manual_resolution_support.rs"]
mod manual_resolution_support;
#[path = "manual_resolution.rs"]
mod manual_resolution_tests;
#[path = "payload_mutation_support.rs"]
mod payload_mutation_support;
#[path = "resource_lanes.rs"]
mod resource_lanes_tests;
#[path = "saga.rs"]
mod saga_tests;
#[path = "side_effect_support.rs"]
mod side_effect_support;
#[path = "side_effects.rs"]
mod side_effects_tests;
use self::fact_retention_support::*;
use self::manual_resolution_support::{
    append_run_state_commit, compensate_saga_policy, manual_authorization,
    manual_resolution_artifacts_from_verified, manual_resolution_payload_from_verified,
    manual_resolution_recorded_for_run, manual_resolution_request,
    manual_resolution_request_from_verified, manual_saga_policy, prepared_manual_resolution_commit,
    proof_manual_saga_policy, run_state_preconditions, saga_preconditions,
    verified_manual_resolution_for_run_seq, verified_manual_resolution_for_seq,
};
use self::payload_mutation_support::{
    certify_payloads_for_policy, set_attempt_failure_node_attempt, set_payload_spec_hash,
    set_remediation_purpose, set_side_effect_ledger, set_side_effect_node_attempt,
};
use self::side_effect_support::*;

#[derive(Debug, Default)]
struct StoreContractRunStore {
    inner: AsyncInMemoryRunStore,
    projection: ProjectionSnapshot,
    authorities: BTreeMap<RunId, CertifiedRunStoreAuthority>,
}

impl StoreContractRunStore {
    fn new() -> Self {
        Self::default()
    }

    fn append_test_commit_plan(
        &mut self,
        plan: PreparedCommitPlan,
    ) -> std::result::Result<CommitOutcome, StoreError> {
        let bundle = test_bundle_from_plan(plan)?;
        self.append_test_commit_bundle(bundle)
    }

    fn append_test_commit_bundle(
        &mut self,
        bundle: PreparedCommitBundle,
    ) -> std::result::Result<CommitOutcome, StoreError> {
        let request = bundle.request();
        let request_run_id = request.run_id().clone();
        let request_authority = request.preconditions().certified_run_authority.clone();
        self.inner
            .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
        let outcome = poll_ready_store_future(self.inner.append_prepared_commit_bundle(bundle));
        if outcome.is_ok() {
            self.projection = self
                .inner
                .projection_snapshot()
                .expect("in-memory projection snapshot");
            if let Some(authority) = request_authority {
                self.authorities.insert(request_run_id, authority);
            }
        }
        outcome
    }

    fn expected_next_seq(&self, run_id: &RunId) -> StreamSeq {
        poll_ready_store_future(self.inner.expected_next_seq(run_id)).expect("expected next seq")
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
        poll_ready_store_future(self.inner.load_run_stream(run_id)).expect("load run stream")
    }

    fn projection_snapshot(&self) -> &ProjectionSnapshot {
        &self.projection
    }

    fn certified_authority(&self, run_id: &RunId) -> &CertifiedRunStoreAuthority {
        self.authorities
            .get(run_id)
            .expect("test run must have certified authority")
    }

    fn certified_preconditions(&self, run_id: &RunId) -> CommitPreconditions {
        CommitPreconditions {
            certified_run_authority: Some(self.certified_authority(run_id).clone()),
            ..CommitPreconditions::default()
        }
    }

    fn certified_spec_hash(&self, run_id: &RunId) -> SpecHash {
        self.certified_authority(run_id).spec_hash().clone()
    }

    fn certify_payloads_for_run(&self, run_id: &RunId, payloads: &mut [KernelEventPayload]) {
        let spec_hash = self.certified_spec_hash(run_id);
        for payload in payloads {
            set_payload_spec_hash(payload, &spec_hash);
        }
    }
}

fn side_effect_pair_id() -> SideEffectPairId {
    side_effect_pair_id_for_contract(&default_side_effect_contract())
}

fn side_effect_pair_id_for_contract(contract: &spec::SideEffectContractSpec) -> SideEffectPairId {
    spec::side_effect_pair_id(&submit_node_id(), &side_effect_output_cell(), contract)
        .expect("certified side-effect pair id")
}

fn remediation_pair_id() -> SideEffectPairId {
    spec::side_effect_pair_id(
        &remediation_submit_node_id(),
        &remediation_output_cell(),
        &default_side_effect_contract(),
    )
    .expect("certified remediation side-effect pair id")
}

fn submit_node_id() -> NodeId {
    node_id(70)
}

fn submit_attempt_id() -> AttemptId {
    attempt_id(72)
}

fn verify_node_id() -> NodeId {
    node_id(170)
}

fn verify_attempt_id() -> AttemptId {
    attempt_id(172)
}

fn remediation_submit_node_id() -> NodeId {
    node_id(180)
}

fn remediation_submit_attempt_id() -> AttemptId {
    attempt_id(182)
}

fn remediation_verify_node_id() -> NodeId {
    node_id(190)
}

fn remediation_verify_attempt_id() -> AttemptId {
    attempt_id(192)
}

fn side_effect_output_cell() -> CellId {
    cell_id(78)
}

fn remediation_output_cell() -> CellId {
    cell_id(178)
}

fn default_side_effect_contract() -> spec::SideEffectContractSpec {
    side_effect_contract_with_verification(spec::SideEffectVerificationSpec::Finalized { depth: 1 })
}

fn side_effect_contract_with_verification(
    verification: spec::SideEffectVerificationSpec,
) -> spec::SideEffectContractSpec {
    spec::SideEffectContractSpec {
        contract_digest: content_digest(77),
        resource_claim: spec::ResourceClaimSpec::ManualOnly,
        verification,
    }
}

fn saga_authority_spec(policy: SagaPolicySpec) -> spec::TypedExecutionSpec {
    saga_authority_spec_with_verification(
        policy,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

fn saga_authority_spec_with_verification(
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

fn saga_authority_spec_with_authoring_config_hash(
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

fn run_id(byte: u8) -> RunId {
    run_id_with_saga_policy(byte, &SagaPolicySpec::NoSideEffects)
}

fn run_id_with_saga_policy(byte: u8, policy: &SagaPolicySpec) -> RunId {
    run_identity_material_for_saga_policy(byte, policy)
        .derive_run_id()
        .expect("test run id")
}

fn run_identity_material_for_saga_policy(
    byte: u8,
    policy: &SagaPolicySpec,
) -> events::RunIdentityMaterialV1 {
    let authority_spec = saga_authority_spec(policy.clone());
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    run_identity_material_for_test(certified_spec_hash, &store_scope_hex(byte))
}

fn execution_claim_scope(byte: u8) -> ExecutionClaimScope {
    ExecutionClaimScope::from_run_identity_material(&run_identity_material_for_saga_policy(
        byte,
        &SagaPolicySpec::NoSideEffects,
    ))
}

fn run_identity_material_for_run_id(
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

fn run_identity_material_for_spec_run_id(
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

fn fact_run_id(byte: u8) -> RunId {
    fact_run_id_for_node(byte, node_id(90))
}

fn fact_run_id_for_node(byte: u8, fact_node_id: NodeId) -> RunId {
    let certified_spec_hash = fact_authority_spec_with_node(fact_node_id)
        .spec_hash()
        .expect("fact authority spec hash");
    run_identity_material_for_test(certified_spec_hash, &store_scope_hex(byte))
        .derive_run_id()
        .expect("fact test run id")
}

fn store_scope_hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(16)
}

fn side_effect_pair_role(
    role: events::SideEffectPairRole,
) -> (SideEffectPairId, events::SideEffectPairRole) {
    (side_effect_pair_id(), role)
}

fn run_admitted(run_id: RunId) -> KernelEventPayload {
    run_admitted_with_saga_policy(run_id, &SagaPolicySpec::NoSideEffects)
}

fn run_admitted_with_saga_policy(
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

fn fact_authority_spec_with_node(fact_node_id: NodeId) -> spec::TypedExecutionSpec {
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

fn run_admitted_with_fact_descriptor_for_node(
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

fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new("mfm.test:portfolio_snapshot:1")
            .expect("entry-point op id"),
        entry_point_registry_digest: content_digest(30),
    }
}

fn state_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        attempt_no: 1,
        state_kind: state_kind(12),
        state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
    })
}

fn append_default_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: impl AsRef<str>,
    payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
) -> std::result::Result<CommitOutcome, StoreError> {
    store.append_prepared_commit(default_commit_request(
        run_id,
        store.expected_next_seq(run_id),
        commit_key,
        payloads,
        required_artifacts,
    ))
}

fn default_commit_request(
    run_id: &RunId,
    expected_next_seq: StreamSeq,
    commit_key: impl AsRef<str>,
    payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
) -> CommitRequest {
    typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: expected_next_seq,
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: required_artifacts,
        preconditions: CommitPreconditions::default(),
    }
}

fn append_state_attempt_started(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> std::result::Result<CommitOutcome, StoreError> {
    append_default_commit(
        store,
        run_id,
        commit_key,
        vec![state_attempt_started()],
        Vec::new(),
    )
}

fn append_side_effect_attempt_started(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> std::result::Result<CommitOutcome, StoreError> {
    append_default_commit(
        store,
        run_id,
        commit_key,
        vec![side_effect_attempt_started()],
        Vec::new(),
    )
}

fn append_state_attempt_interrupted(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> std::result::Result<CommitOutcome, StoreError> {
    append_default_commit(
        store,
        run_id,
        commit_key,
        vec![state_attempt_interrupted()],
        Vec::new(),
    )
}

fn state_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        output_cell_id: cell_id(21),
    })
}

fn state_attempt_interrupted() -> KernelEventPayload {
    state_attempt_interrupted_for(node_id(20), attempt_id(23))
}

fn state_attempt_interrupted_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
    })
}

fn cell_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        cell_id: cell_id(21),
        scope_id: scope_id(22),
        attempt_id: attempt_id(23),
        semantic_type_id: semantic_id("position", 24),
        schema_id: schema_id("mfm.test.position", 25),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(26),
        },
        context: spec::CellContextSpec::no_context(),
        artifact_id,
        content_digest: digest,
        evidence_hash: evidence.evidence_hash().expect("cell evidence hash"),
        producer_state_kind: None,
        producer_state_version: None,
    })
}

fn terminal_cell_commit_payloads(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> Vec<KernelEventPayload> {
    vec![
        cell_produced(artifact_id, digest),
        state_attempt_completed(),
    ]
}

fn public_output_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
    KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        receipt_cell_id: cell_id(21),
        public_schema_id: schema_id("mfm.test.public_output", 3),
        output_spec_digest: content_digest(27),
        cells: vec![events::NamedTypedCellRef {
            public_field_path: PublicFieldPath::new("result").expect("field path"),
            cell_id: cell_id(21),
            producer: CellProducer::Node(node_id(20)),
            scope_id: scope_id(22),
            semantic_type_id: semantic_id("position", 24),
            schema_id: schema_id("mfm.test.position", 25),
            value_lineage: ValueLineageRef {
                lineage_digest: content_digest(26),
            },
            content_digest: digest,
            artifact_id,
            evidence_hash: evidence.evidence_hash().expect("public cell evidence hash"),
        }],
        rendered_digest: content_digest(28),
        rendered_artifact_id: None,
        rendered_artifact_evidence_hash: None,
        renderer_descriptor_id: descriptor_id(29),
    })
}

fn public_output_produced_with_rendered_artifact(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    rendered_artifact_id: ArtifactId,
    rendered_digest: ContentDigest,
) -> KernelEventPayload {
    let mut payload = public_output_produced(artifact_id, digest);
    let KernelEventPayload::PublicOutputProduced(public_output) = &mut payload else {
        unreachable!("helper returns public-output payload");
    };
    let rendered =
        public_output_artifact_ref(rendered_artifact_id.clone(), rendered_digest.clone());
    public_output.rendered_digest = rendered_digest;
    public_output.rendered_artifact_id = Some(rendered_artifact_id);
    public_output.rendered_artifact_evidence_hash =
        Some(rendered.evidence_hash().expect("rendered evidence hash"));
    payload
}

fn event_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> events::ArtifactEvidenceRef {
    let store = store_artifact_ref(artifact_id.clone(), digest.clone());
    events::ArtifactEvidenceRef {
        artifact_id,
        role: ArtifactRole::StateOutput,
        schema_id: schema_id("mfm.test.position", 25),
        semantic_type_id: Some(semantic_id("position", 24)),
        content_digest: digest,
        evidence_hash: store.evidence_hash().expect("event artifact evidence hash"),
        byte_len: 128,
        media_type: media_type("application/json"),
    }
}

fn store_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.position", 25)),
        semantic_type_id: Some(semantic_id("position", 24)),
        producer_node_id: Some(node_id(20)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn public_output_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.public_output", 3)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(20)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::PublicOutput,
    }
}

fn intent_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    intent_artifact_ref_for_node(artifact_id, digest, submit_node_id())
}

fn remediation_intent_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> ArtifactEvidenceRef {
    intent_artifact_ref_for_node(artifact_id, digest, remediation_submit_node_id())
}

fn intent_artifact_ref_for_node(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    producer_node_id: NodeId,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.side_effect_intent", 70)),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::SideEffectIntent,
    }
}

fn side_effect_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    artifact_role: ArtifactRole,
) -> ArtifactEvidenceRef {
    side_effect_artifact_ref_for_nodes(
        artifact_id,
        digest,
        schema_id,
        artifact_role,
        submit_node_id(),
        verify_node_id(),
    )
}

fn remediation_side_effect_evidence(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    artifact_role: ArtifactRole,
) -> ArtifactEvidenceRef {
    side_effect_artifact_ref_for_nodes(
        artifact_id,
        digest,
        schema_id,
        artifact_role,
        remediation_submit_node_id(),
        remediation_verify_node_id(),
    )
}

fn side_effect_artifact_ref_for_nodes(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    artifact_role: ArtifactRole,
    submit_node_id: NodeId,
    verify_node_id: NodeId,
) -> ArtifactEvidenceRef {
    let producer_node_id = match artifact_role {
        ArtifactRole::Receipt | ArtifactRole::Confirmation | ArtifactRole::AmbiguityEvidence => {
            verify_node_id
        }
        _ => submit_node_id,
    };
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None::<SeedId>,
        artifact_role,
    }
}

fn fact_attempt_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id: attempt_id(91),
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("fact_failed").expect("error code"),
            category: events::ErrorCategory::Runtime,
            retryable,
            safe_message: "fact state failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn run_completed_for_run(
    run_id: RunId,
    outcome: events::RunCompletionOutcome,
) -> KernelEventPayload {
    KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id,
        spec_hash: spec_hash(1),
        outcome,
    })
}

fn run_completed_for_run_with_policy(
    run_id: RunId,
    outcome: events::RunCompletionOutcome,
    policy: SagaPolicySpec,
) -> KernelEventPayload {
    let spec_hash = saga_authority_spec(policy)
        .spec_hash()
        .expect("run completion saga authority spec hash");
    let mut payload = run_completed_for_run(run_id, outcome);
    let KernelEventPayload::RunCompleted(inner) = &mut payload else {
        unreachable!("helper returns run completed payload");
    };
    inner.spec_hash = spec_hash;
    payload
}

fn completed_outcome(byte: u8) -> events::RunCompletionOutcome {
    events::RunCompletionOutcome::Completed(Box::new(events::PublicOutputCompletionEvidence {
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        public_output_event_id: event_id(byte),
    }))
}

trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: CommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome>;

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome>;
}

impl TestPreparedCommitExt for StoreContractRunStore {
    fn append_prepared_commit(
        &mut self,
        request: CommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }
}

fn append_async_prepared_commit(
    store: &AsyncInMemoryRunStore,
    request: CommitRequest,
) -> mfm_store::v1::Result<CommitOutcome> {
    let plan = test_prepared_commit_plan(request.clone(), request.required_artifacts().to_vec())?;
    store.seed_artifact_evidence_for_test(plan.admitted_artifacts())?;
    let bundle = test_bundle_from_plan(plan)?;
    poll_ready_store_future(store.append_prepared_commit_bundle(bundle))
}

fn async_expected_next_seq(store: &AsyncInMemoryRunStore, run_id: &RunId) -> StreamSeq {
    poll_ready_store_future(store.expected_next_seq(run_id)).expect("async expected next seq")
}

fn run_start_request(run_id: RunId, commit_key: &str) -> CommitRequest {
    run_start_request_with_saga_policy(run_id, commit_key, &SagaPolicySpec::NoSideEffects)
}

fn fact_run_start_request_with_node(
    run_id: RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> CommitRequest {
    let authority_spec = fact_authority_spec_with_node(fact_node_id.clone());
    let descriptor_artifact = fact_descriptor_artifact_ref();
    CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(commit_key).expect("commit key"),
        vec![run_admitted_with_fact_descriptor_for_node(
            run_id.clone(),
            fact_node_id,
        )],
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            descriptor_artifact,
        ],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("fact run start request")
}

fn fact_run_start_bundle_with_node(
    run_id: RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> PreparedCommitBundle {
    let request = fact_run_start_request_with_node(run_id, commit_key, fact_node_id);
    let descriptor_artifact = fact_descriptor_artifact_ref();
    let artifacts = CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        request.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    let plan = PreparedCommitPlan::from(
        PreparedCommit::<RunAdmission>::new(request, artifacts).expect("fact run admission"),
    );
    PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_descriptor_bytes(), descriptor_artifact.clone())
                .expect("descriptor bytes"),
        ],
        vec![
            existing_artifact_admission(&spec_artifact_ref()),
            existing_artifact_admission(&certificate_artifact_ref()),
        ],
    )
    .expect("fact run start bundle")
}

fn existing_artifact_admission(evidence: &ArtifactEvidenceRef) -> ExistingArtifactAdmission {
    ExistingArtifactAdmission::new(
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("evidence hash"),
    )
}

fn run_start_request_with_saga_policy(
    run_id: RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) -> CommitRequest {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(commit_key).expect("commit key"),
        vec![run_admitted_with_saga_policy(run_id.clone(), saga_policy)],
        vec![spec_artifact_ref(), certificate_artifact_ref()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("run start request")
}

fn ensure_test_run_admitted(store: &mut StoreContractRunStore, run_id: &RunId, commit_key: &str) {
    if store.expected_next_seq(run_id) == StreamSeq::FIRST {
        store
            .append_prepared_commit(run_start_request(run_id.clone(), commit_key))
            .expect("append fixture run admission");
    }
}

fn admitted_store(run_id: &RunId, commit_key: &str) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), commit_key))
        .expect("append run start");
    store
}

fn admitted_fact_store(run_id: &RunId, commit_key: &str) -> StoreContractRunStore {
    admitted_fact_store_with_node(run_id, commit_key, node_id(90))
}

fn admitted_fact_store_with_node(
    run_id: &RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    store
        .append_test_commit_bundle(fact_run_start_bundle_with_node(
            run_id.clone(),
            commit_key,
            fact_node_id,
        ))
        .expect("append fact run start");
    store
}

fn admitted_store_with_saga_policy(
    run_id: &RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    append_run_start_with_saga_policy(&mut store, run_id, commit_key, saga_policy);
    store
}

fn append_run_start_with_saga_policy(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) {
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            commit_key,
            saga_policy,
        ))
        .expect("append run start");
}

fn append_side_effect_prepare(store: &mut StoreContractRunStore, run_id: &RunId) {
    ensure_test_run_admitted(store, run_id, "sidefx-fixture-run-start");
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let intent_evidence = intent_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-attempt-start",
        vec![side_effect_attempt_started()],
        Vec::new(),
    )
    .expect("append sidefx attempt start");
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-prepare",
        vec![
            side_effect_intent(artifact_id, artifact_digest),
            side_effect_claim(),
            side_effect_prepared(1, "token-1"),
        ],
        vec![intent_evidence],
    )
    .expect("append sidefx prepare");
}

fn append_side_effect_prepare_for_ledger(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
) {
    append_side_effect_prepare_for_ledger_on_attempt(
        store,
        run_id,
        commit_key,
        ledger_key,
        resource_key,
        artifact_byte,
        start_attempt,
        node_id(70),
        attempt_id(72),
    );
}

#[allow(clippy::too_many_arguments)]
fn append_side_effect_prepare_for_ledger_on_attempt(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    ensure_test_run_admitted(store, run_id, "sidefx-ledger-fixture-run-start");
    if start_attempt {
        append_certified_side_effect_commit(
            store,
            run_id,
            &format!("{commit_key}-attempt-start"),
            vec![side_effect_attempt_started_for(
                node_id.clone(),
                attempt_id.clone(),
            )],
            Vec::new(),
        )
        .expect("append sidefx attempt start");
    }

    let fixture = side_effect_prepare_fixture_for_ledger(
        ledger_key,
        resource_key,
        artifact_byte,
        node_id,
        attempt_id,
    );
    append_certified_side_effect_commit(
        store,
        run_id,
        commit_key,
        fixture.payloads,
        fixture.required_artifacts,
    )
    .expect("append sidefx prepare");
}

fn append_side_effect_started(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-started",
        vec![side_effect_started("owner-1", 1, "token-1")],
        Vec::new(),
    )
    .expect("append sidefx started");
}

fn append_side_effect_verify_attempt_started(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-verify-attempt-start",
        vec![side_effect_verify_attempt_started()],
        Vec::new(),
    )
    .expect("append sidefx verify attempt start");
}

fn append_side_effect_observation_setup(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_prepare(store, run_id);
    append_side_effect_started(store, run_id);
    append_side_effect_verify_attempt_started(store, run_id);
}

fn append_side_effect_failure_setup(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_prepare(store, run_id);
    append_side_effect_verify_attempt_started(store, run_id);
}

fn append_remediation_attempts_started(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) {
    for (suffix, payload) in [
        (
            "submit",
            side_effect_attempt_started_for(
                remediation_submit_node_id(),
                remediation_submit_attempt_id(),
            ),
        ),
        (
            "verify",
            side_effect_attempt_started_for(
                remediation_verify_node_id(),
                remediation_verify_attempt_id(),
            ),
        ),
    ] {
        append_certified_side_effect_commit(
            store,
            run_id,
            &format!("{commit_key}-{suffix}"),
            vec![payload],
            Vec::new(),
        )
        .expect("append remediation side-effect attempt start");
    }
}

fn append_certified_side_effect_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    mut payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
) -> std::result::Result<CommitOutcome, StoreError> {
    store.certify_payloads_for_run(run_id, &mut payloads);
    store.append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: required_artifacts,
        preconditions: store.certified_preconditions(run_id),
    })
}

fn append_fact_recorded_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> std::result::Result<CommitOutcome, StoreError> {
    append_fact_recorded_commit_for_height(store, run_id, commit_key, 850000)
}

fn append_fact_recorded_commit_for_height(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    height: u64,
) -> std::result::Result<CommitOutcome, StoreError> {
    let response = fact_artifact_ref_for_height(height);
    let mut payloads = vec![fact_recorded(&response)];
    store.certify_payloads_for_run(run_id, &mut payloads);
    let mut preconditions = store.certified_preconditions(run_id);
    preconditions.required_run_state = RequiredRunState::Started;
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone()],
        preconditions: preconditions,
    };
    let plan = test_prepared_commit_plan(request, vec![response.clone()])?;
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_response_bytes_for_height(height), response)
                .expect("fact response bytes"),
        ],
        Vec::new(),
    )?;
    store.append_test_commit_bundle(bundle)
}

fn append_generic_nonretryable_failure(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    key_prefix: &str,
) {
    ensure_test_run_admitted(store, run_id, &format!("{key_prefix}-fixture-run-start"));
    store
        .append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-start")).expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: store.certified_preconditions(run_id),
        })
        .expect("append generic attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-failed")).expect("commit key"),
            payloads: vec![fact_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: store.certified_preconditions(run_id),
        })
        .expect("append generic attempt failure");
}

fn append_confirmed_side_effect_attempt_failures(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) {
    let mut submit_failure = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut submit_failure, submit_node_id(), submit_attempt_id());
    append_certified_side_effect_commit(
        store,
        run_id,
        commit_key,
        vec![submit_failure, side_effect_attempt_failed(false)],
        Vec::new(),
    )
    .expect("append confirmed side-effect attempt failures");
}

fn append_manual_blocked_forward_failure(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    failure_commit_key: &str,
) {
    append_forward_confirmation(store, run_id);
    append_confirmed_side_effect_attempt_failures(store, run_id, failure_commit_key);
}

fn append_forward_confirmation(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_observation_setup(store, run_id);
    let submission_artifact_id = artifact_id(84);
    let submission_digest = content_digest(85);
    let receipt_artifact_id = artifact_id(86);
    let receipt_digest = content_digest(87);
    let confirmation_artifact_id = artifact_id(88);
    let confirmation_digest = content_digest(89);
    append_certified_side_effect_commit(
        store,
        run_id,
        "forward-confirmation",
        vec![
            side_effect_submission_observed(
                submission_artifact_id.clone(),
                submission_digest.clone(),
            ),
            side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone()),
            side_effect_confirmation(
                confirmation_artifact_id.clone(),
                confirmation_digest.clone(),
            ),
        ],
        vec![
            side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
            side_effect_evidence(
                confirmation_artifact_id,
                confirmation_digest,
                confirmation_schema(),
                ArtifactRole::Confirmation,
            ),
        ],
    )
    .expect("append forward confirmation");
}

fn append_forward_receipt(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_observation_setup(store, run_id);
    let submission_artifact_id = artifact_id(84);
    let submission_digest = content_digest(85);
    let receipt_artifact_id = artifact_id(86);
    let receipt_digest = content_digest(87);
    append_certified_side_effect_commit(
        store,
        run_id,
        "forward-receipt",
        vec![
            side_effect_submission_observed(
                submission_artifact_id.clone(),
                submission_digest.clone(),
            ),
            side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone()),
        ],
        vec![
            side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
        ],
    )
    .expect("append forward receipt");
}

fn append_remediation_confirmation(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    ledger_key: events::SideEffectLedgerKey,
) {
    append_remediation_attempts_started(store, run_id, "remediation-attempts-started");
    let intent_artifact_id = artifact_id(101);
    let intent_digest = content_digest(102);
    let submission_artifact_id = artifact_id(103);
    let submission_digest = content_digest(104);
    let receipt_artifact_id = artifact_id(105);
    let receipt_digest = content_digest(106);
    let confirmation_artifact_id = artifact_id(107);
    let confirmation_digest = content_digest(108);

    let mut intent = side_effect_intent(intent_artifact_id.clone(), intent_digest.clone());
    let mut claim = side_effect_claim();
    let mut prepared = side_effect_prepared(1, "token-1");
    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut submission =
        side_effect_submission_observed(submission_artifact_id.clone(), submission_digest.clone());
    let mut receipt = side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone());
    let mut confirmation = side_effect_confirmation(
        confirmation_artifact_id.clone(),
        confirmation_digest.clone(),
    );
    for payload in [
        &mut intent,
        &mut claim,
        &mut prepared,
        &mut started,
        &mut submission,
        &mut receipt,
        &mut confirmation,
    ] {
        set_remediation_purpose(payload, ledger_key.clone());
    }
    let payloads = vec![
        intent,
        claim,
        prepared,
        started,
        submission,
        receipt,
        confirmation,
    ];
    append_certified_side_effect_commit(
        store,
        run_id,
        "remediation-confirmation",
        payloads,
        vec![
            remediation_intent_artifact_ref(intent_artifact_id, intent_digest),
            remediation_side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            remediation_side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
            remediation_side_effect_evidence(
                confirmation_artifact_id,
                confirmation_digest,
                confirmation_schema(),
                ArtifactRole::Confirmation,
            ),
        ],
    )
    .expect("append remediation confirmation");
}

fn side_effect_evidence(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    side_effect_artifact_ref(artifact_id, digest, schema_id, role)
}

fn assert_projection_codecs_round_trip(snapshot: &ProjectionSnapshot) {
    use mfm_store::v1::codec;

    for (_run_id, state) in snapshot.run_states() {
        assert_eq!(
            codec::parse_run_state(codec::run_state_str(*state)).expect("parse run state"),
            *state
        );
    }
    for (run_id, projection) in snapshot.run_completions() {
        let json = codec::run_completion_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_run_completion_projection(&json).expect("parse run completion"),
            (run_id.clone(), projection.clone())
        );
    }
    for (run_id, projection) in snapshot.saga_engagements() {
        let json = codec::saga_engagement_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_saga_engagement_projection(&json).expect("parse saga engagement"),
            (run_id.clone(), projection.clone())
        );
    }
    for (run_id, projection) in snapshot.manual_resolutions() {
        let json = codec::manual_resolution_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_manual_resolution_projection(&json).expect("parse manual resolution"),
            (run_id.clone(), projection.clone())
        );
    }
    for (_key, projection) in snapshot.attempts() {
        let json = codec::attempt_projection_json(projection);
        assert_eq!(
            codec::parse_attempt_projection(&json).expect("parse attempt"),
            projection.clone()
        );
    }
    for (run_id, cell_id, projection) in snapshot.cells() {
        let json = codec::cell_projection_json(run_id, cell_id, projection);
        assert_eq!(
            codec::parse_cell_projection(&json).expect("parse cell"),
            ((run_id.clone(), cell_id.clone()), projection.clone())
        );
    }
    for (_ledger_ref, projection) in snapshot.side_effects() {
        let json = codec::side_effect_projection_json(projection);
        assert_eq!(
            codec::parse_side_effect_projection(&json).expect("parse side effect"),
            projection.clone()
        );
    }
    for (lane_key, projection) in snapshot.resource_lanes() {
        let json = codec::resource_lane_projection_json(lane_key, projection);
        assert_eq!(
            codec::parse_resource_lane_projection(&json).expect("parse resource lane"),
            (lane_key.clone(), projection.clone())
        );
    }
    for (run_id, schema_id, projection) in snapshot.public_outputs() {
        let json = codec::public_output_projection_json(run_id, schema_id, projection);
        assert_eq!(
            codec::parse_public_output_projection(&json).expect("parse public output"),
            ((run_id.clone(), schema_id.clone()), projection.clone())
        );
    }
    for (_run_id, projection) in snapshot.retentions() {
        for manifest in projection.manifests.values() {
            let json = codec::retention_manifest_projection_json(manifest);
            assert_eq!(
                codec::parse_retention_manifest_projection(&json)
                    .expect("parse retention manifest"),
                manifest.clone()
            );
        }
    }

    let synthetic_run_id = run_id(210);
    let run_completion = RunCompletionProjection {
        event_id: event_id(211),
        outcome: completed_outcome(212),
    };
    let json = codec::run_completion_projection_json(&synthetic_run_id, &run_completion);
    assert_eq!(
        codec::parse_run_completion_projection(&json).expect("parse synthetic run completion"),
        (synthetic_run_id.clone(), run_completion)
    );

    let saga_engagement = SagaEngagementProjection {
        event_id: event_id(213),
        reason: SagaEngagementReason::ForwardAmbiguous {
            pair_id: side_effect_pair_id(),
        },
    };
    let json = codec::saga_engagement_projection_json(&synthetic_run_id, &saga_engagement);
    assert_eq!(
        codec::parse_saga_engagement_projection(&json).expect("parse synthetic saga engagement"),
        (synthetic_run_id.clone(), saga_engagement)
    );

    let manual_resolution = ManualResolutionProjection {
        event_id: event_id(215),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.test.manual_evidence", 219),
        evidence_hash: content_digest(220),
        evidence_artifact_id: artifact_id(221),
        authorization_schema_id: schema_id("mfm.test.manual_authorization", 222),
        authorization_hash: content_digest(223),
        authorization_artifact_id: artifact_id(224),
        note: Some(events::ManualResolutionNote::new("operator reviewed").expect("note")),
    };
    let json = codec::manual_resolution_projection_json(&synthetic_run_id, &manual_resolution);
    assert_eq!(
        codec::parse_manual_resolution_projection(&json)
            .expect("parse synthetic manual resolution"),
        (synthetic_run_id, manual_resolution)
    );
}
