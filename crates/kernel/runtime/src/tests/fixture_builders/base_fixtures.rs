use super::*;

pub(in crate::tests::support) fn fixture() -> Fixture {
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
    let framework_effect = mfm_effects::Pure::descriptor().expect("framework effect");
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
        effect_kind: framework_effect.kind.clone(),
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
                effect_kind: framework_effect.kind,
                effect_class: framework_effect.class.as_str().to_owned(),
                effect_name: framework_effect.name.to_owned(),
                effect_version: framework_effect.version,
                capabilities: no_caps,
                runner: "pure".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                effect_contract_digest: None,
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
