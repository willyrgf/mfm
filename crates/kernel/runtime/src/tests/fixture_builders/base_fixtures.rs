use super::*;

pub(in crate::tests::support) fn fixture() -> Fixture {
    certified_base_fixture(false)
}

pub(in crate::tests::support) fn certified_base_fixture(with_fact_read: bool) -> Fixture {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<RuntimeSourceState>()
        .expect("source registration");
    states
        .register::<RuntimeReadState>()
        .expect("read registration");
    states
        .register::<RuntimeFactReadState>()
        .expect("fact read registration");

    let seed = CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed");
    let seed_digest = seed.content_digest().clone();
    let seed_byte_len = seed.byte_len() as u64;
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let input = root.seed(mfm_program::SeedKey::new("initial")?, seed.clone())?;
            let produced = root.scope().state::<RuntimeSourceState, _>(
                StateKey::new("a")?,
                NoContext,
                CertifierConfig { multiplier: 3 },
                input,
            )?;
            let result = if with_fact_read {
                root.scope().state::<RuntimeFactReadState, _>(
                    StateKey::new("b")?,
                    NoContext,
                    CertifierConfig { multiplier: 5 },
                    produced,
                )?
            } else {
                root.scope().state::<RuntimeReadState, _>(
                    StateKey::new("b")?,
                    NoContext,
                    CertifierConfig { multiplier: 5 },
                    produced,
                )?
            };
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &FixturePublicOutputs { result },
            )
        },
    )
    .expect("base runtime draft");
    let certified =
        mfm_certify::certify_program_draft(&draft).expect("certified base runtime spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    fixture_from_base_runtime_spec(runtime_spec, seed_digest, seed_byte_len, with_fact_read)
}

fn fixture_from_base_runtime_spec(
    runtime_spec: CertifiedRuntimeSpec,
    seed_digest: ContentDigest,
    seed_byte_len: u64,
    with_fact_read: bool,
) -> Fixture {
    let seed = runtime_spec.spec().seeds.first().expect("seed");
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
    let node_a = runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.source");
    let node_b = runtime_node_by_descriptor_name(
        &runtime_spec,
        if with_fact_read {
            "mfm.runtime.test.fact_read"
        } else {
            "mfm.runtime.test.read_output"
        },
    );
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
