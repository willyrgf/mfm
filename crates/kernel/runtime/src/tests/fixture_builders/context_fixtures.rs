use super::*;

pub(in crate::tests::support) fn fixture_with_context_bound_states() -> Fixture {
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

pub(in crate::tests::support) fn fixture_from_context_runtime_spec(
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

pub(in crate::tests::support) fn fixture_from_runtime_spec(
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

pub(in crate::tests::support) fn runtime_node_by_descriptor_name(
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
