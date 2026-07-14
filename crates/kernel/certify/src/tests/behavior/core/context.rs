use super::*;

#[test]
fn certifies_context_bound_transition_graph() {
    let draft = context_bound_draft(false);
    let certified = certify_program_draft(&draft).expect("context-bound draft certifies");
    let typed = certified.validated_spec().spec();
    assert_eq!(typed.contexts.len(), 1);
    let source = node_with_descriptor_name(typed, "mfm.certify.test.context_source");
    let source_output = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == source.output_cell)
        .expect("source output cell");
    assert!(matches!(
        &source_output.context,
        spec::CellContextSpec::Bound {
            resource_kind,
            stage,
            producer,
            ..
        } if resource_kind == &context_resource_kind()
            && stage == &context_stage()
            && producer.producer_descriptor_ids.as_slice()
                == std::slice::from_ref(&source.descriptor_id)
            && !producer.seed_producers_allowed
    ));
    let consumer = node_with_descriptor_name(typed, "mfm.certify.test.context_consumer");
    assert!(matches!(
        &consumer.input_bindings.root,
        spec::InputBindingNodeSpec::Cell(cell)
            if matches!(cell.context, spec::InputContextSpec::Required { .. })
    ));
}

#[test]
fn certification_rejects_context_table_entry_that_fails_registered_typed_decode() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let invalid_context =
        PlainCanonicalJsonBytes::from_json_str(r#"{"network":7}"#).expect("canonical context");
    let old_context_ref = typed.contexts[0].context_ref.clone();
    let new_context_ref = {
        let context = &mut typed.contexts[0];
        context.canonical_context = invalid_context;
        context.canonical_context_digest = context.canonical_context.content_digest();
        context.canonical_context_byte_len = context.canonical_context.as_bytes().len() as u64;
        context.context_ref = spec::CertifiedContextSpec::derive_context_ref(
            &context.context_descriptor_id,
            &context.schema_id,
            &context.semantic_type_id,
            &context.canonicalizer_identity,
            &context.canonical_context,
        )
        .expect("context ref");
        context.context_ref.clone()
    };
    retarget_context_ref(&mut typed, &old_context_ref, &new_context_ref);

    let error = certify_untrusted_typed_spec(typed, &registry)
        .expect_err("registered context descriptor must decode context payload");

    assert_problem_contains(
        error,
        ProblemClass::InvalidDataShape,
        "registered descriptor",
    );
}

#[test]
fn certification_rejects_context_bound_output_under_wrong_node_context() {
    let (registry, mut typed) = context_bound_registry_and_spec(true);
    let other_context_ref = typed.contexts[1].context_ref.clone();
    let source_output_cell = node_with_descriptor_name(&typed, "mfm.certify.test.context_source")
        .output_cell
        .clone();
    let output = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == source_output_cell)
        .expect("source output cell");
    let spec::CellContextSpec::Bound { context_ref, .. } = &mut output.context else {
        panic!("source output must be context-bound");
    };
    *context_ref = other_context_ref;

    let error = certify_untrusted_typed_spec(typed, &registry)
        .expect_err("wrong output context must reject");
    assert_invalid_semantic_contains(error, "output context");
}

#[test]
fn certification_rejects_no_context_state_consuming_context_bound_resource() {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ContextSourceState>()
        .expect("source state registration");
    states
        .register::<MultiplyState>()
        .expect("multiply state registration");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let context = root.scope().declare_context(ContractContext {
                network: "primary".to_owned(),
            })?;
            let produced = root.scope().state::<ContextSourceState, _>(
                StateKey::new("context-source")?,
                &context,
                TestConfig { multiplier: 3 },
                seed,
            )?;
            let consumed = root.scope().state::<MultiplyState, _>(
                StateKey::new("plain-consumer")?,
                NoContext,
                TestConfig { multiplier: 5 },
                produced,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs { result: consumed },
            )
        },
    )
    .expect("draft builds before certification");

    let error = certify_program_draft(&draft).expect_err("no-context consumer must reject");
    assert_invalid_semantic_contains(error, "without a descriptor contract");
}

#[test]
fn certification_rejects_input_context_that_does_not_match_source_cell() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let consumer = node_with_descriptor_name_mut(&mut typed, "mfm.certify.test.context_consumer");
    let spec::InputBindingNodeSpec::Cell(cell) = &mut consumer.input_bindings.root else {
        panic!("consumer input must be a cell");
    };
    cell.context = spec::InputContextSpec::no_context();
    consumer.input_bindings.digest =
        content_digest_json(input_node_json(&consumer.input_bindings.root)).expect("input digest");

    let error =
        certify_untrusted_typed_spec(typed, &registry).expect_err("input context must reject");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidInterfaceWiring),
        "{error}"
    );
    assert!(error.to_string().contains("cell context"), "{error}");
}

#[test]
fn certification_rejects_unauthorized_context_bound_seed() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let context_ref = typed.contexts[0].context_ref.clone();
    let seed_cell_id = typed.seeds[0].cell_id.clone();
    let seed_cell = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == seed_cell_id)
        .expect("seed cell");
    seed_cell.context = spec::CellContextSpec::Bound {
        context_ref,
        resource_kind: context_resource_kind(),
        stage: context_stage(),
        producer: Box::new(spec::ContextProducerSpec {
            producer_descriptor_ids: vec![context_source_descriptor_id().expect("descriptor id")],
            seed_producers_allowed: false,
        }),
    };

    let error =
        certify_untrusted_typed_spec(typed, &registry).expect_err("seed context must reject");
    assert_invalid_semantic_contains(error, "not authorized to produce context-bound cell");
}

#[test]
fn verifies_persisted_spec_certificate_parts() {
    let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    let verified = verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    assert_eq!(verified.spec_hash(), certified.spec_hash());
    assert_eq!(verified.certificate_hash(), certified.certificate_hash());
}

#[test]
fn certificate_evidence_mismatches_reject_persisted_parts() {
    #[derive(Clone, Copy)]
    enum Mismatch {
        RegistryDigest,
        DescriptorIdentity,
        DescriptorDigest,
        SpecHash,
    }

    let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    for (case, expected) in [
        (Mismatch::RegistryDigest, "registry digest mismatch"),
        (Mismatch::DescriptorIdentity, "descriptor identity mismatch"),
        (Mismatch::DescriptorDigest, "descriptor digest mismatch"),
        (Mismatch::SpecHash, "spec hash mismatch"),
    ] {
        let mut evidence = certified.certificate().evidence.clone();
        match case {
            Mismatch::RegistryDigest => {
                evidence.registry_digest =
                    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x72));
            }
            Mismatch::DescriptorIdentity => {
                evidence.descriptor_identities[0].descriptor_id =
                    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x73));
            }
            Mismatch::DescriptorDigest => {
                evidence.descriptor_identities[0].descriptor_digest =
                    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x74));
            }
            Mismatch::SpecHash => {
                evidence.spec_hash =
                    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x75));
            }
        }
        let certificate = CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&certificate),
            &registry,
        )
        .expect_err(expected);
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }
}

#[test]
fn parsed_persisted_parts_are_untrusted_until_verifier_succeeds() {
    let (_registry, _certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    let untrusted = persisted_parts
        .parse_untrusted()
        .expect("parsed untrusted persisted parts");
    assert_eq!(
        untrusted.spec().spec_hash().expect("untrusted spec hash"),
        untrusted.certificate().evidence.spec_hash
    );
    let error = verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &CertificationRegistry::new(),
    )
    .expect_err("parsed persisted parts need registry-backed verifier success");
    assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
}

#[test]
fn typed_certification_rejects_problem_taxonomy() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.cells.push(spec.cells[0].clone());
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = spec
                .nodes
                .iter_mut()
                .find(|node| {
                    matches!(
                        node.input_bindings.root,
                        spec::InputBindingNodeSpec::Cell(_)
                    )
                })
                .expect("cell-input node");
            let first_input = match &mut node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell,
                _ => panic!("expected cell input"),
            };
            first_input.schema_id = SchemaId::new(
                "mfm.certify.test.wrong",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x44),
            )
            .expect("schema id");
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.nodes[0].effect_kind = ApplySideEffect::descriptor()
                .expect("side effect descriptor")
                .kind;
        },
        ProblemClass::InvalidDataShape => |spec| {
            spec.config_refs.clear();
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let seed_lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| matches!(lineage.producer, spec::CellProducer::Seed(_)))
                .expect("seed lineage");
            seed_lineage.config_ref_digest = Some(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x55),
            ));
        },
        ProblemClass::InvalidTerminalShape => |spec| {
            spec.public_outputs.outputs.clear();
        },
    );
}
