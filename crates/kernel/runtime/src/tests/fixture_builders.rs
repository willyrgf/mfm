use super::*;

pub(super) fn fixture() -> Fixture {
    let scope = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0);
    let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, D1);
    let seed_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2);
    let node_a = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D3);
    let cell_a = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D4);
    let node_b = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D5);
    let cell_b = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D6);
    let descriptor_a = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D7);
    let descriptor_b = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let semantic = fixture_value_semantic_id();
    let value_schema = fixture_value_schema_id();
    let input_schema = SchemaId::new("mfm.test.input", "1", DigestAlgorithm::Sha256JcsV1, DB)
        .expect("input schema");
    let config_schema = SchemaId::new("mfm.test.config", "1", DigestAlgorithm::Sha256JcsV1, DC)
        .expect("config schema");
    let public_schema = SchemaId::new("mfm.test.public", "1", DigestAlgorithm::Sha256JcsV1, DD)
        .expect("public schema");
    let effect_kind =
        EffectKind::new("mfm.test", "pure", DigestAlgorithm::Sha256JcsV1, DE).expect("effect");
    let read_effect =
        EffectKind::new("mfm.test", "read", DigestAlgorithm::Sha256JcsV1, DF).expect("read effect");
    let cap_kind = CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
        .expect("cap kind");
    let cap_version = CapabilityVersion::new("mfm.cap.read_db.v1").expect("cap version");
    let adapter_kind = AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
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
    let config_digest = digest_for_bytes(TEST_CONFIG_BYTES);
    let config_ref = spec::ConfigRef {
        schema_id: config_schema.clone(),
        artifact_id: ArtifactId::from_digest(config_digest.algorithm(), *config_digest.digest()),
        digest: config_digest,
        byte_len: TEST_CONFIG_BYTES.len() as u64,
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
    let seed_digest = digest_for_bytes(TEST_SEED_BYTES);
    let seed_ref = events::SeedCellRef {
        seed_id: seed_id.clone(),
        cell_id: seed_cell.clone(),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        digest: seed_digest.clone(),
        seed_artifact: seed_cell_artifact_evidence(
            &seed_id,
            value_schema.clone(),
            semantic.clone(),
            seed_digest.clone(),
            TEST_SEED_BYTES.len() as u64,
        ),
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
    let public_output_cell = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("result").expect("field"),
        cell_id: cell_b.clone(),
        producer: spec::CellProducer::Node(node_b.clone()),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        value_lineage: lineage_b.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    let public_outputs = spec::PublicOutputSpec {
        public_schema_id: public_schema.clone(),
        outputs: vec![public_output_cell.clone()],
        renderer_descriptor: renderer.clone(),
    };
    let output_spec_digest = public_outputs.digest().expect("public output digest");
    let render_node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let render_config_ref = spec::framework_config_ref("public_output_render", &render_node)
        .expect("render config ref");
    let render_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    let render_descriptor = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let receipt_schema = spec::public_output_receipt_schema_id().expect("receipt schema");
    let receipt_semantic =
        spec::public_output_receipt_semantic_type_id().expect("receipt semantic");
    let render_lineage = spec::ValueLineageRef {
        lineage_digest: content(0x45),
    };
    let render_input_root = spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
        field_path: public_output_cell.public_field_path.clone(),
        node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: public_output_cell.public_field_path.clone(),
            cell_id: public_output_cell.cell_id.clone(),
            semantic_type_id: public_output_cell.semantic_type_id.clone(),
            schema_id: public_output_cell.schema_id.clone(),
            required_terminal: public_output_cell.required_terminal,
            value_lineage: public_output_cell.value_lineage.clone(),
            context: spec::InputContextSpec::no_context(),
        })),
    }]);
    let render_input_binding = spec::InputBindingSpec {
        input_schema_id: public_schema.clone(),
        input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DC),
        digest: content_digest_json(input_node_json(&render_input_root))
            .expect("render input digest"),
        root: render_input_root,
    };
    let managed_effect = ManagedPlatformWrite::descriptor().expect("managed effect");
    let render_state_kind = StateKind::new(
        "mfm.framework.state",
        "render_public_outputs",
        DigestAlgorithm::Sha256JcsV1,
        DD,
    )
    .expect("render state kind");
    let render_state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .expect("render state version");
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
    let render_node_spec = spec::NodeSpec {
        node_id: render_node.clone(),
        stable_key: spec::StableAuthorKey::new("public-output").expect("render key"),
        scope_id: scope.clone(),
        state_kind: render_state_kind.clone(),
        state_version: render_state_version.clone(),
        descriptor_id: render_descriptor.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: render_config_ref.clone(),
        input_bindings: render_input_binding,
        output_cell: render_cell.clone(),
        effect_kind: managed_effect.kind.clone(),
        capability_bindings: no_caps.clone(),
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
            spec::PublicOutputRenderNodeSpec {
                public_schema_id: public_schema.clone(),
                output_spec_digest: output_spec_digest.clone(),
                renderer_descriptor: renderer.clone(),
                required_cells: public_outputs.outputs.clone(),
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: planning.clone(),
        deterministic_predecessors: vec![node_b.clone()],
    };
    let mut spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D4),
                name: "mfm.test.composition".to_owned(),
                version: "mfm.test.composition.v1".to_owned(),
            },
            config_hash: content(0x60),
        },
        saga: spec::SagaPolicySpec::NoSideEffects,
        contexts: Vec::new(),
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
            required_digest: Some(seed_digest),
        }],
        descriptor_identities: vec![
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &node_a_spec,
                descriptor_a.clone(),
                "mfm.test.state.a",
                effect_kind,
                no_caps.clone(),
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
            spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                descriptor_id: render_descriptor.clone(),
                name: "mfm.framework.render_public_outputs".to_owned(),
                state_kind: render_state_kind,
                state_version: render_state_version,
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id: render_config_ref.schema_id.clone(),
                input_schema_id: public_schema.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed_effect.kind,
                effect_class: managed_effect.class.as_str().to_owned(),
                effect_name: managed_effect.name.to_owned(),
                effect_version: managed_effect.version,
                capabilities: no_caps,
                runner: "managed_platform_write".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                side_effect_contract_digest: None,
            })),
            spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
        ],
        config_refs: vec![config_ref.clone(), render_config_ref],
        nodes: vec![
            render_node_spec.clone(),
            node_b_spec.clone(),
            node_a_spec.clone(),
        ],
        remediations: BTreeMap::new(),
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
                context: spec::CellContextSpec::no_context(),
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
                context: spec::CellContextSpec::no_context(),
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
                context: spec::CellContextSpec::no_context(),
            },
            spec::CellSpec {
                cell_id: render_cell.clone(),
                producer: spec::CellProducer::Node(render_node.clone()),
                scope_id: scope.clone(),
                semantic_type_id: receipt_semantic,
                schema_id: receipt_schema,
                value_lineage: render_lineage.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::PublicOutputArtifact,
                redaction_policy: spec::RedactionPolicy::Public,
                context: spec::CellContextSpec::no_context(),
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
                scope_id: scope.clone(),
                producer: spec::CellProducer::Node(node_b.clone()),
                input_cells: vec![cell_a.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
            spec::ValueLineage {
                lineage_ref: render_lineage,
                scope_id: scope,
                producer: spec::CellProducer::Node(render_node.clone()),
                input_cells: vec![cell_b.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
        ],
        planning_lineage: Vec::new(),
        public_outputs,
    })
    .expect("typed spec");
    append_runtime_retention_lifecycle_node(&mut spec, render_cell.clone(), true);
    let retention_receipt = runtime_retention_receipt_cell(&spec);
    append_runtime_complete_lifecycle_node(&mut spec, retention_receipt, true);
    append_runtime_resolve_saga_terminal_lifecycle_node(&mut spec);
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .expect("envelope");
    let runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    Fixture {
        runtime_spec,
        run_id,
        store_scope_id: fixture_store_scope_id(),
        invocation_key_digest: content(0x10),
        seed_ref,
        descriptor_a,
        descriptor_b,
        descriptor_c: None,
        render_node,
        render_cell,
        cell_a,
        cell_b,
        cell_c: None,
        cap_kind,
        cap_version,
        adapter_kind,
        adapter_version,
    }
}

pub(super) fn fixture_with_first_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_a.clone())
}

pub(super) fn fixture_with_read_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_b.clone())
}

pub(super) fn fixture_with_read_node_and_other_node_fact_descriptors() -> (
    Fixture,
    mfm_facts::FactDescriptor,
    mfm_facts::FactDescriptor,
) {
    let (mut fixture, read_descriptor, _read_ref) = fixture_with_read_node_fact_descriptor();
    let other_descriptor = test_fact_descriptor_with_kind("mfm.runtime.test.other_fact");
    let other_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&other_descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("other fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![other_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("other fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![other_ref];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, read_descriptor, other_descriptor)
}

pub(super) fn fixture_with_node_fact_descriptor(
    select_output_cell: impl FnOnce(&Fixture) -> CellId,
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    let mut fixture = fixture();
    let output_cell = select_output_cell(&fixture);
    let descriptor =
        <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor");
    let descriptor_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == output_cell)
        .expect("fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![descriptor_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![descriptor_ref.clone()];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, descriptor, descriptor_ref)
}

#[derive(Clone, Copy)]
pub(super) enum RuntimeSideEffectClaim {
    ManualOnly,
    Exclusive,
    ExactTouchedSet,
}

impl RuntimeSideEffectClaim {
    fn into_resource_claim(self) -> ResourceClaim {
        match self {
            Self::ManualOnly => ResourceClaim::manual_only(),
            Self::Exclusive => ResourceClaim::exclusive(
                exclusive_resource_namespace(),
                <CertifierValue as mfm_values::MfmValue>::schema_id()
                    .expect("exclusive key schema"),
            ),
            Self::ExactTouchedSet => ResourceClaim::exact_touched_set(
                exact_touched_set_resource_namespace(),
                <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                    .expect("touched-set evidence schema"),
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RuntimeSideEffectFixtureShape {
    Chained,
    IndependentSecond,
    CompensatingPairWithFailingTail,
}

pub(super) fn runtime_side_effect_fixture(
    shape: RuntimeSideEffectFixtureShape,
    claim: RuntimeSideEffectClaim,
    verification: spec::SideEffectVerificationSpec,
) -> Fixture {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<RuntimeSubmitAState>()
        .expect("submit a registration");
    states
        .register::<RuntimeSubmitBState>()
        .expect("submit b registration");
    states
        .register::<RuntimeReadState>()
        .expect("read registration");
    states
        .register::<RuntimeSeedReadState>()
        .expect("seed read registration");
    states
        .register::<RuntimeTailState>()
        .expect("tail registration");
    let seed = CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed");
    let seed_digest = seed.content_digest().clone();
    let seed_byte_len = seed.byte_len() as u64;
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            if matches!(
                shape,
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail
            ) {
                root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
                    on_remediation_unresolved:
                        mfm_program::RemediationUnresolved::FailWithoutAcdcClaim,
                })?;
            } else {
                root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            }
            let input = root.seed(mfm_program::SeedKey::new("initial")?, seed.clone())?;
            match shape {
                RuntimeSideEffectFixtureShape::Chained => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        NoContext,
                        CertifierConfig { multiplier: 3 },
                        input,
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let result = root.scope().state::<RuntimeReadState, _>(
                        StateKey::new("b")?,
                        NoContext,
                        CertifierConfig { multiplier: 5 },
                        forward.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
                RuntimeSideEffectFixtureShape::IndependentSecond => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        NoContext,
                        CertifierConfig { multiplier: 3 },
                        input.clone(),
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let side_effect = forward.into_handle();
                    let result = root.scope().state::<RuntimeSeedReadState, _>(
                        StateKey::new("b")?,
                        NoContext,
                        CertifierConfig { multiplier: 5 },
                        input,
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &DualFixturePublicOutputs {
                            result,
                            side_effect,
                        },
                    )
                }
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail => {
                    let (forward_a, _remediation_a) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitAState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            NoContext,
                            NoContext,
                            SideEffectNodeParams {
                                key: StateKey::new("a")?,
                                config: CertifierConfig { multiplier: 3 },
                                input,
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-a")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let (forward_b, _remediation_b) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitBState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            NoContext,
                            NoContext,
                            SideEffectNodeParams {
                                key: StateKey::new("b")?,
                                config: CertifierConfig { multiplier: 7 },
                                input: forward_a.into_handle(),
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-b")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification,
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let result = root.scope().state::<RuntimeTailState, _>(
                        StateKey::new("c")?,
                        NoContext,
                        CertifierConfig { multiplier: 1 },
                        forward_b.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
            }
        },
    )
    .expect("side-effect draft");
    let certified = mfm_certify::certify_program_draft(&draft).expect("certified side-effect spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    fixture_from_runtime_spec(shape, runtime_spec, seed_digest, seed_byte_len)
}

pub(super) fn fixture_with_context_bound_states() -> Fixture {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<RuntimeContextSourceState>()
        .expect("context source registration");
    states
        .register::<RuntimeContextConsumerState>()
        .expect("context consumer registration");
    let seed = CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed");
    let seed_digest = seed.content_digest().clone();
    let seed_byte_len = seed.byte_len() as u64;
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let input = root.seed(mfm_program::SeedKey::new("initial")?, seed.clone())?;
            let context = root.scope().declare_context(RuntimeContractContext {
                network: "primary".to_owned(),
            })?;
            let produced = root.scope().state::<RuntimeContextSourceState, _>(
                StateKey::new("context-source")?,
                &context,
                CertifierConfig { multiplier: 3 },
                input,
            )?;
            let result = root.scope().state::<RuntimeContextConsumerState, _>(
                StateKey::new("context-consumer")?,
                &context,
                CertifierConfig { multiplier: 5 },
                produced,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &CertifierPublicOutputs { result },
            )
        },
    )
    .expect("context-bound draft");
    let certified = mfm_certify::certify_program_draft(&draft).expect("certified context spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    fixture_from_context_runtime_spec(runtime_spec, seed_digest, seed_byte_len)
}

pub(super) fn fixture_from_context_runtime_spec(
    runtime_spec: CertifiedRuntimeSpec,
    seed_digest: ContentDigest,
    seed_byte_len: u64,
) -> Fixture {
    let spec = runtime_spec.spec();
    let seed = spec.seeds.first().expect("seed");
    let seed_ref = events::SeedCellRef {
        seed_id: seed.seed_id.clone(),
        cell_id: seed.cell_id.clone(),
        scope_id: seed.scope_id.clone(),
        semantic_type_id: seed.semantic_type_id.clone(),
        schema_id: seed.schema_id.clone(),
        digest: seed_digest.clone(),
        seed_artifact: seed_cell_artifact_evidence(
            &seed.seed_id,
            seed.schema_id.clone(),
            seed.semantic_type_id.clone(),
            seed_digest,
            seed_byte_len,
        ),
    };
    let node_a = runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.context_source");
    let node_b =
        runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.context_consumer");
    let render = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("render node");
    let render_node = render.node_id.clone();
    let render_cell = render.output_cell.clone();
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    let adapter_binding = runtime_adapter_binding();
    Fixture {
        runtime_spec,
        run_id,
        store_scope_id: fixture_store_scope_id(),
        invocation_key_digest: content(0x10),
        seed_ref,
        descriptor_a: node_a.descriptor_id.clone(),
        descriptor_b: node_b.descriptor_id.clone(),
        descriptor_c: None,
        render_node,
        render_cell,
        cell_a: node_a.output_cell.clone(),
        cell_b: node_b.output_cell.clone(),
        cell_c: None,
        cap_kind: RuntimeReadCap::kind().expect("read cap kind"),
        cap_version: RuntimeReadCap::version().expect("read cap version"),
        adapter_kind: adapter_binding.adapter_kind,
        adapter_version: adapter_binding.adapter_version,
    }
}

pub(super) fn fixture_from_runtime_spec(
    shape: RuntimeSideEffectFixtureShape,
    runtime_spec: CertifiedRuntimeSpec,
    seed_digest: ContentDigest,
    seed_byte_len: u64,
) -> Fixture {
    let spec = runtime_spec.spec();
    let seed = spec.seeds.first().expect("seed");
    let seed_ref = events::SeedCellRef {
        seed_id: seed.seed_id.clone(),
        cell_id: seed.cell_id.clone(),
        scope_id: seed.scope_id.clone(),
        semantic_type_id: seed.semantic_type_id.clone(),
        schema_id: seed.schema_id.clone(),
        digest: seed_digest.clone(),
        seed_artifact: seed_cell_artifact_evidence(
            &seed.seed_id,
            seed.schema_id.clone(),
            seed.semantic_type_id.clone(),
            seed_digest,
            seed_byte_len,
        ),
    };
    let node_a = runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.submit_a");
    let node_b_name = match shape {
        RuntimeSideEffectFixtureShape::Chained => "mfm.runtime.test.read_output",
        RuntimeSideEffectFixtureShape::IndependentSecond => "mfm.runtime.test.read_seed",
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail => {
            "mfm.runtime.test.submit_b"
        }
    };
    let node_b = runtime_node_by_descriptor_name(&runtime_spec, node_b_name);
    let node_c = matches!(
        shape,
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail
    )
    .then(|| runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.tail"));
    let render = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("render node");
    let render_node = render.node_id.clone();
    let render_cell = render.output_cell.clone();
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    let adapter_binding = runtime_adapter_binding();
    Fixture {
        runtime_spec,
        run_id,
        store_scope_id: fixture_store_scope_id(),
        invocation_key_digest: content(0x10),
        seed_ref,
        descriptor_a: node_a.descriptor_id.clone(),
        descriptor_b: node_b.descriptor_id.clone(),
        descriptor_c: node_c.as_ref().map(|node| node.descriptor_id.clone()),
        render_node,
        render_cell,
        cell_a: node_a.output_cell.clone(),
        cell_b: node_b.output_cell.clone(),
        cell_c: node_c.map(|node| node.output_cell),
        cap_kind: RuntimeReadCap::kind().expect("read cap kind"),
        cap_version: RuntimeReadCap::version().expect("read cap version"),
        adapter_kind: adapter_binding.adapter_kind,
        adapter_version: adapter_binding.adapter_version,
    }
}

pub(super) fn runtime_node_by_descriptor_name(
    runtime_spec: &CertifiedRuntimeSpec,
    name: &str,
) -> spec::NodeSpec {
    runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            runtime_spec
                .state_descriptor_for_node(node)
                .expect("state descriptor")
                .name
                == name
        })
        .cloned()
        .unwrap_or_else(|| panic!("node for descriptor {name}"))
}

pub(super) async fn drive_until_public_output_produced(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..8 {
        drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive until public output");
        let projections = store.projection_snapshot();
        if projections.run_state(&fixture.run_id) == store::RunState::Started
            && matches!(
                projections.public_output(
                    &fixture.run_id,
                    &fixture.runtime_spec.spec().public_outputs.public_schema_id,
                ),
                Some(store::PublicOutputProjection::Produced { .. })
            )
        {
            return;
        }
    }
    panic!("public output was not produced");
}

pub(super) fn fixture_with_first_managed_write_state() -> Fixture {
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
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

pub(super) fn fixture_with_first_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(super) fn fixture_with_first_exclusive_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(super) fn fixture_with_first_finalized_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(super) fn fixture_with_first_exact_touched_set_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(super) fn fixture_with_first_exact_touched_set_finalized_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(super) fn fixture_with_manual_resolution_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_side_effect_state();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let manual = spec::ManualResolutionEvidenceSpec {
        evidence_schema: fixture.seed_ref.schema_id.clone(),
        authorization: manual_authorization(0xe0),
    };
    envelope.spec.saga = spec::SagaPolicySpec::ManualResolution { manual };
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

pub(super) fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

pub(super) fn fixture_with_independent_second_node_and_first_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(super) fn fixture_with_independent_second_node_and_first_exclusive_finalized_side_effect_state(
) -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(super) fn fixture_with_two_side_effects_and_failing_tail() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(super) struct NodeSpecFixture {
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

pub(super) fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
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

pub(super) fn state_descriptor(
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
        effect_class: runner.to_owned(),
        effect_name: runner.to_owned(),
        effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
        capabilities,
        runner: runner.to_owned(),
        emitted_fact_descriptors: Vec::new(),
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

pub(super) fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
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

pub(super) fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

pub(super) fn stable_domain_key_ref_json(key: &spec::StableDomainKeyRef) -> serde_json::Value {
    serde_json::json!({
        "content_digest": key.content_digest.as_str(),
        "schema_id": key.schema_id.as_str(),
    })
}
