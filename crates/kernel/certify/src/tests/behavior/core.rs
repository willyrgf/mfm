use super::*;

#[test]
fn certification_registry_rejects_duplicate_state_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<MultiplyState>()
        .expect("first state descriptor registers");

    let error = registry
        .register_state::<ConflictingMultiplyState>()
        .expect_err("conflicting state kind/version rejects");

    assert_invalid_semantic_contains(error, "state kind/version");
}

#[test]
fn certification_registry_rejects_duplicate_operation_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_operation::<MultiplyOperation>()
        .expect("first operation descriptor registers");

    let error = registry
        .register_operation::<ConflictingMultiplyOperation>()
        .expect_err("conflicting operation kind/version rejects");

    assert_invalid_semantic_contains(error, "operation kind/version");
}

#[test]
fn registered_config_validator_rejects_invalid_config_bytes() {
    enum Case {
        SchemaShapeMismatch,
        NoncanonicalBytes,
        NonAuthoritativeCanonicalEncoding,
    }

    for case in [
        Case::SchemaShapeMismatch,
        Case::NoncanonicalBytes,
        Case::NonAuthoritativeCanonicalEncoding,
    ] {
        match case {
            Case::SchemaShapeMismatch => {
                let registry = registered_multiply_certification_registry();
                let invalid = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":"bad"}"#)
                    .expect("canonical invalid config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&invalid);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, invalid.as_bytes())
                    .expect_err("invalid typed config shape rejects");
                assert!(err
                    .to_string()
                    .contains("typed config did not match registered schema"));
            }
            Case::NoncanonicalBytes => {
                let registry = registered_multiply_certification_registry();
                let canonical = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":2}"#)
                    .expect("canonical config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&canonical);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, br#"{ "multiplier": 2 }"#)
                    .expect_err("noncanonical config rejects");
                let rendered = err.to_string();
                assert!(rendered.contains("typed config"));
                assert!(rendered.contains("not normalized canonical JSON"));
            }
            Case::NonAuthoritativeCanonicalEncoding => {
                let mut registry = CertificationRegistry::new();
                registry
                    .insert_config_validator(
                        config_validator_for::<DefaultedConfig>().expect("validator"),
                    )
                    .expect("insert validator");
                let supplied = PlainCanonicalJsonBytes::from_json_str(r#"{}"#)
                    .expect("canonical but not authoritative config");
                let config_ref = config_ref_for_bytes::<DefaultedConfig>(&supplied);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, supplied.as_bytes())
                    .expect_err("non-authoritative canonical config rejects");
                assert!(err
                    .to_string()
                    .contains("did not match registered canonical encoding"));
            }
        }
    }
}

#[test]
fn trusted_draft_config_ref_accepts_exact_bytes_without_descriptor_validator() {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("draft registry");
    let config = draft
        .state_nodes()
        .first()
        .expect("state node")
        .config
        .clone();
    let config_ref = spec::ConfigRef {
        schema_id: config.schema_id.clone(),
        artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *config.content_digest.digest(),
        ),
        digest: config.content_digest.clone(),
        byte_len: config.byte_len as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };

    let source = registry
        .validate_config_ref_bytes(&config_ref, config.canonical_json.as_bytes())
        .expect("trusted exact config validates");
    assert_eq!(source, Some(ConfigValidationSource::TrustedExactRef));
}

#[test]
fn certifies_reference_program_draft() {
    let draft = reference_draft();
    let expected_public_schema = draft.public_output_spec().public_schema_id().clone();
    let certified = certify_program_draft(&draft).expect("certified");
    certified.envelope().verify_hash().expect("hash verifies");
    certified
        .certificate()
        .verify_hash()
        .expect("certificate hash verifies");
    assert_eq!(
        certified.certificate().evidence.certifier_algorithm,
        CERTIFIER_ALGORITHM
    );
    assert_eq!(
        certified.certificate().evidence.spec_hash,
        *certified.spec_hash()
    );
    assert_eq!(
        certified.certificate_hash().as_str(),
        "content:sha256-jcs-v1:9df4bb9724a7c4c452319e57e897cfee33c88120ce9a2f98a25a16f8f42c6b97"
    );
    assert_eq!(
        certified.envelope().spec.public_outputs.public_schema_id,
        expected_public_schema
    );
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    )));
}

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

#[test]
fn typed_spec_requires_registry_authority() {
    let spec = certify_program_draft(&reference_draft())
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    let error =
        certify_untrusted_typed_spec(spec, &CertificationRegistry::new()).expect_err("must reject");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_node_fact_descriptor_outside_registered_descriptor() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        spec.nodes[0]
            .fact_descriptor_allowlist
            .push(spec::FactDescriptorRef {
                descriptor_hash: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0xa1),
                ),
            });
    });
}

#[test]
fn certification_carries_derive_fact_descriptor_allowlist_authority() {
    let draft = fact_emitting_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let certified = certify_program_draft(&draft).expect("certified");
    let expected_ref = mfm_program::fact_descriptor_ref::<ChainHeadFact>().expect("descriptor ref");
    let derived_descriptor = ChainHeadFact::descriptor().expect("derived descriptor");
    assert_eq!(
        mfm_program::facts::fact_descriptor_hash(&derived_descriptor).expect("descriptor hash"),
        expected_ref.descriptor_hash
    );

    let spec = certified.validated_spec().spec();
    let fact_node = spec
        .nodes
        .iter()
        .find(|node| {
            node.fact_descriptor_allowlist.as_slice() == std::slice::from_ref(&expected_ref)
        })
        .expect("fact-emitting node");
    let descriptor_identity = spec
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_ref())
            }
            _ => None,
        })
        .expect("fact-emitting descriptor identity");
    assert_eq!(
        descriptor_identity.emitted_fact_descriptors.as_slice(),
        std::slice::from_ref(&expected_ref)
    );

    let baseline_hash = certified.spec_hash().clone();
    let mut mutated = spec.clone();
    let mutated_node = mutated
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == fact_node.descriptor_id)
        .expect("mutated fact node");
    mutated_node.fact_descriptor_allowlist.clear();
    let mutated_descriptor = mutated
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_mut())
            }
            _ => None,
        })
        .expect("mutated descriptor identity");
    mutated_descriptor.emitted_fact_descriptors.clear();

    assert_ne!(
        mutated.spec_hash().expect("mutated spec hash"),
        baseline_hash
    );
    let error = certify_untrusted_typed_spec(mutated, &registry)
        .expect_err("tampered allow-list authority rejects");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_forged_framework_descriptor_bypass() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.forged".to_owned();
        forged.state_kind = state_kind_json("forged", serde_json::json!({ "framework": "forged" }))
            .expect("state kind");
        forged.state_version =
            StateVersion::new("mfm.framework.state.forged.v1").expect("state version");
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_accepts_valid_lifecycle_framework_node_shapes() {
    let (registry, spec) = reference_registry_and_spec();

    certify_untrusted_typed_spec(spec, &registry).expect("lifecycle framework nodes certify");
}

#[test]
fn certification_mints_descriptor_set_and_lifecycle_views() {
    let certified = certify_program_draft(&reference_draft()).expect("certified");
    let spec = certified.validated_spec().spec();
    let graph = certified.validated_spec().graph();
    let descriptors = certified.descriptor_set();
    let lifecycle = certified.framework_lifecycle();

    assert_eq!(graph.scope_count(), spec.scopes.len());
    assert_eq!(graph.config_ref_count(), spec.config_refs.len());
    assert_eq!(graph.value_lineage_count(), spec.value_lineages.len());
    assert_eq!(graph.cell_count(), spec.cells.len());
    assert_eq!(graph.forward_node_count(), spec.nodes.len());
    assert_eq!(graph.remediation_count(), spec.remediations.len());
    assert_eq!(graph.descriptors(), descriptors);
    assert!(descriptors.state_descriptors().count() > 0);
    assert!(descriptors
        .renderer(&spec.public_outputs.renderer_descriptor.descriptor_id)
        .is_some());
    assert!(descriptors
        .state(lifecycle.render().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.retention().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.complete().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.resolve().descriptor_id())
        .is_some());
    assert!(graph.cell(lifecycle.render().output_cell()).is_some());
    assert!(graph.forward_node(lifecycle.render().node_id()).is_some());
    assert_ne!(
        lifecycle.render().node_id(),
        lifecycle.retention().node_id()
    );
    assert_ne!(
        lifecycle.retention().output_cell(),
        lifecycle.complete().output_cell()
    );
}

#[test]
fn certification_rejects_missing_retention_lifecycle_node() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        });
    });
}

#[test]
fn certification_rejects_forged_lifecycle_framework_variant() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let forged = spec
            .nodes
            .iter()
            .find_map(|node| match &node.framework {
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                    Some(retention.clone())
                }
                _ => None,
            })
            .expect("retention metadata");
        let node = spec
            .nodes
            .iter_mut()
            .find(|node| node.framework.is_none())
            .expect("user node");
        node.framework = Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(forged));
    });
}

#[test]
fn certification_rejects_lifecycle_descriptor_mismatch() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.complete_run".to_owned();
        forged.state_kind = state_kind_json(
            "complete_run",
            serde_json::json!({ "framework": "complete_run" }),
        )
        .expect("complete state kind");
        forged.state_version = StateVersion::new("mfm.framework.state.complete_run.v1")
            .expect("complete state version");
        forged.runner = "forged".to_owned();
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_rejects_invalid_lifecycle_ordering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let public_cell = spec
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone();
        append_retention_lifecycle_node(spec, public_cell);
    });
}

#[test]
fn certification_rejects_lifecycle_framework_permissions() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.adapter_bindings.push(spec::AdapterBinding {
            adapter_kind: AdapterKind::new(
                "mfm.certify.test",
                "forged-adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xa1),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.certify.test.adapter.v1")
                .expect("adapter version"),
            binding_digest: None,
        });
    });
}

#[test]
fn certification_rejects_user_consumers_of_lifecycle_receipts() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            let render_receipt = lifecycle_render_receipt_cell(spec);
            append_user_receipt_consumer(spec, render_receipt, "user/render-receipt");
        },
        ProblemClass::InvalidTopology => |spec| {
            let retention_receipt = lifecycle_retention_receipt_cell(spec);
            append_user_receipt_consumer(spec, retention_receipt, "user/retention-receipt");
        },
    );
}

#[test]
fn certification_rejects_executable_nodes_outside_lifecycle_tail() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        append_independent_user_node(spec, "user/outside-lifecycle-tail");
    });
}

#[test]
fn certification_rejects_forged_lifecycle_config_ref() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.config_ref.byte_len += 1;
    });
}

#[test]
fn certification_rejects_builtin_lifecycle_descriptors_without_framework_metadata() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ProjectRetentionManifestNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::CompleteRunNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ResolveSagaTerminalNodeSpec>(spec);
        },
    );
}

#[test]
fn certification_rejects_forged_lifecycle_input_binding() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("public_output_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::CompleteRunNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let input_cell = spec.cells.first().expect("input cell").clone();
            let node = find_lifecycle_node_mut::<spec::ResolveSagaTerminalNodeSpec>(spec);
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                            .expect("field path"),
                        cell_id: input_cell.cell_id,
                        semantic_type_id: input_cell.semantic_type_id,
                        schema_id: input_cell.schema_id,
                        required_terminal: spec::RequiredTerminal::ProducedOnly,
                        value_lineage: input_cell.value_lineage,
                        context: spec::InputContextSpec::no_context(),
                    })),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
    );
}

#[test]
fn certification_rejects_operation_input_binding_tampering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let frame = spec
            .planning_lineage
            .first_mut()
            .expect("operation lineage frame");
        let input_cell = match &mut frame.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell,
            _ => panic!("expected operation cell input"),
        };
        input_cell.cell_id = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x91));
        frame.input_bindings.digest =
            content_digest_json(input_node_json(&frame.input_bindings.root)).expect("input digest");
        frame.input_binding_digest = frame.input_bindings.digest.clone();
    });
}

#[test]
fn certification_rejects_stable_id_and_scope_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes[0].stable_key =
                spec::StableAuthorKey::new("renamed-node").expect("node key");
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.scopes[0].parent_scope_id = Some(spec.scopes[0].scope_id.clone());
        },
    );
}

#[test]
fn certification_rejects_framework_lineage_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidDataMeaning => |spec| {
            spec.planning_lineage.clear();
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let lineage = spec
                .nodes
                .first_mut()
                .expect("node with operation planning lineage");
            lineage.planning_lineage.active_operation_instances = vec![
                OperationInstanceId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x92),
                ),
            ];
            lineage.planning_lineage.lineage_digest = planning_lineage_digest(
                &lineage.planning_lineage.active_operation_instances,
                &lineage.planning_lineage.completed_operation_frames,
            )
            .expect("planning lineage digest");
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let render_node_id = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                    )
                })
                .expect("render node")
                .node_id
                .clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(render_node_id.clone())
                })
                .expect("render lineage");
            lineage.input_cells.clear();
        },
    );
}

#[test]
fn certification_summary_keys_are_stable() {
    let keys = [
        ProblemClass::InvalidTopology.summary_key(),
        ProblemClass::InvalidInterfaceWiring.summary_key(),
        ProblemClass::InvalidSemanticTransition.summary_key(),
        ProblemClass::InvalidDataShape.summary_key(),
        ProblemClass::InvalidDataMeaning.summary_key(),
        ProblemClass::InvalidTerminalShape.summary_key(),
    ];
    assert_eq!(
        keys,
        [
            "invalid_topology_rejected",
            "invalid_interface_wiring_rejected",
            "invalid_semantic_transition_rejected",
            "invalid_data_shape_rejected",
            "invalid_data_meaning_rejected",
            "invalid_terminal_shape_rejected"
        ]
    );
}
