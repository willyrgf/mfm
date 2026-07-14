use super::*;

#[test]
fn root_builder_binds_seed_and_public_output_specs() {
    let draft = build_root(
        ScopeKey::new("portfolio/root").expect("scope key"),
        |root| {
            let seed = launch_seed(42, "cash");
            let handle = root.seed(SeedKey::new("launch-input")?, seed)?;
            let outputs = LaunchPublicOutputs { result: handle };
            root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .expect("root builds");

    assert_eq!(draft.root_key().as_str(), "portfolio/root");
    assert_eq!(draft.scopes().len(), 1);
    assert_eq!(draft.scopes()[0].key.as_str(), "portfolio/root");
    assert_eq!(draft.scopes()[0].scope_id, *draft.root_scope_id());
    assert_eq!(draft.scopes()[0].parent_scope_id, None);
    assert_eq!(draft.seeds().len(), 1);
    assert_eq!(draft.seeds()[0].key.as_str(), "launch-input");
    assert_eq!(draft.seeds()[0].byte_len, 28);
    assert_eq!(
        draft.seeds()[0].content_digest.as_str(),
        "content:sha256-jcs-v1:3247e9eea57a8b2cc5058f1915144ae32ad6142e75b412c530f7d581d5e4c457"
    );
    assert_eq!(
        draft.seeds()[0].schema_id,
        LaunchValue::schema_id().expect("schema id")
    );
    assert_eq!(
        draft.seeds()[0].semantic_type_id,
        LaunchValue::semantic_id().expect("semantic id")
    );

    let public = draft.public_output_spec();
    assert_eq!(public.key().as_str(), "terminal");
    assert_eq!(public.outputs().len(), 1);
    assert_eq!(public.outputs()[0].public_field_path().as_str(), "result");
    assert_eq!(
        public.outputs()[0].cell().cell_id(),
        &draft.seeds()[0].cell_id
    );
    assert_eq!(public.outputs()[0].cell().scope_id(), draft.root_scope_id());
}

#[test]
fn typed_program_launch_plan_collects_config_material() {
    let (state_registry, operation_registry) = multiply_registries();
    let seed = launch_seed(4, "operation");
    let seed_bytes = seed.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry,
        |root| {
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 8 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");
    let expected_len = draft.state_nodes().len() + draft.operation_lineage().len();
    let seed_material =
        std::collections::BTreeMap::from([(draft.seeds()[0].seed_id.clone(), seed_bytes)]);

    let plan = TypedProgramLaunchPlan::from_draft_and_seed_material(draft.clone(), seed_material)
        .expect("launch plan");

    assert_eq!(plan.config_material.len(), expected_len);
    for node in draft.state_nodes() {
        assert!(plan
            .config_material
            .iter()
            .any(|material| material.schema_id == node.config.schema_id
                && material.bytes == node.config.canonical_json));
    }
    for frame in draft.operation_lineage() {
        assert!(plan
            .config_material
            .iter()
            .any(|material| material.schema_id == frame.config.schema_id
                && material.bytes == frame.config.canonical_json));
    }
    assert_eq!(plan.seed_material.len(), 1);
}

#[test]
fn typed_program_launch_plan_matches_seed_material_by_seed_id() {
    let first_seed = launch_seed(1, "first");
    let first_bytes = first_seed.canonical_json().clone();
    let second_seed = launch_seed(2, "second");
    let second_bytes = second_seed.canonical_json().clone();
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, first_seed)?;
        let _second = root.seed(SeedKey::new("second")?, second_seed)?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");
    let first_id = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == "first")
        .expect("first seed spec")
        .seed_id
        .clone();
    let second_id = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == "second")
        .expect("second seed spec")
        .seed_id
        .clone();
    let seeds =
        std::collections::BTreeMap::from([(second_id, second_bytes), (first_id, first_bytes)]);

    let plan = TypedProgramLaunchPlan::from_draft_and_seed_material(draft.clone(), seeds)
        .expect("launch plan");

    let planned_seed_ids = plan
        .seed_material
        .iter()
        .map(|material| material.seed_id.clone())
        .collect::<Vec<_>>();
    let draft_seed_ids = draft
        .seeds()
        .iter()
        .map(|seed| seed.seed_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(planned_seed_ids, draft_seed_ids);
}

#[test]
fn typed_program_launch_plan_rejects_invalid_seed_material() {
    #[derive(Clone, Copy)]
    enum InvalidSeedMaterialCase {
        Missing,
        Unknown,
        Mismatched,
    }

    for (name, case) in [
        ("missing seed material", InvalidSeedMaterialCase::Missing),
        ("unknown seed material", InvalidSeedMaterialCase::Unknown),
        (
            "mismatched seed material",
            InvalidSeedMaterialCase::Mismatched,
        ),
    ] {
        let seed = launch_seed(1, "first");
        let seed_bytes = seed.canonical_json().clone();
        let draft = single_seed_public_output_draft(seed);
        let expected_id = draft.seeds()[0].seed_id.clone();
        let seeds = match case {
            InvalidSeedMaterialCase::Missing => std::collections::BTreeMap::new(),
            InvalidSeedMaterialCase::Unknown => std::collections::BTreeMap::from([
                (expected_id, seed_bytes.clone()),
                (seed_id(0xab), seed_bytes),
            ]),
            InvalidSeedMaterialCase::Mismatched => std::collections::BTreeMap::from([(
                expected_id,
                launch_seed(2, "wrong").canonical_json().clone(),
            )]),
        };

        let error =
            TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seeds).expect_err(name);

        match case {
            InvalidSeedMaterialCase::Missing => assert!(
                matches!(&error, PlanError::Key(message) if message.contains("missing entry-point seed material")),
                "{name}: {error:?}"
            ),
            InvalidSeedMaterialCase::Unknown => assert!(
                matches!(&error, PlanError::Key(message) if message.contains("unknown entry-point seed material")),
                "{name}: {error:?}"
            ),
            InvalidSeedMaterialCase::Mismatched => assert!(
                matches!(&error, PlanError::Canonical(message) if message.contains("did not match draft seed")),
                "{name}: {error:?}"
            ),
        }
    }
}

#[test]
fn registered_state_registry_plans_state_node() {
    let mut registry = StateRegistryBuilder::new();
    register_multiply_state(&mut registry);
    let descriptor = registry
        .snapshot()
        .state_descriptor::<MultiplyState>()
        .expect("registered state descriptor resolves");
    assert_eq!(descriptor.runner(), RunnerKind::Pure);

    let draft = build_root_with_registry(
        ScopeKey::new("portfolio/root").expect("scope key"),
        registry.snapshot(),
        |root| {
            let seed = launch_seed(7, "gross");
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.saga_policy(), &SagaPolicy::NoSideEffects);
    assert_eq!(draft.state_nodes().len(), 1);
    let node = &draft.state_nodes()[0];
    assert_eq!(node.key.as_str(), "multiply");
    assert_eq!(node.scope_id, *draft.root_scope_id());
    assert_eq!(&node.state_descriptor_id, descriptor.descriptor_id());
    assert_eq!(node.runner, RunnerKind::Pure);
    assert_eq!(
        node.config.schema_id,
        LaunchConfig::schema_id().expect("config schema id")
    );
    assert_eq!(
        node.input.input_schema_id,
        LaunchValue::input_schema_id().expect("input schema id")
    );
    assert_eq!(
        node.output_schema_id,
        LaunchValue::schema_id().expect("output schema id")
    );
    assert_eq!(
        node.output_semantic_type_id,
        LaunchValue::semantic_id().expect("output semantic id")
    );
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &node.output_cell_id
    );
}

#[test]
fn declared_context_refs_are_stable_and_duplicates_reject() {
    fn draft_with_context(chain_id: u64) -> TypedProgramDraft {
        build_root(ScopeKey::new("root").expect("scope key"), |root| {
            let input = root.seed(SeedKey::new("input")?, launch_seed(1, "context"))?;
            let _context = root.scope().declare_context(ChainContext {
                chain_id,
                network: "local".to_owned(),
            })?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result: input },
            )
        })
        .expect("root builds")
    }

    let first = draft_with_context(31337);
    let second = draft_with_context(31337);
    let different = draft_with_context(1);
    assert_eq!(first.contexts().len(), 1);
    assert_eq!(
        first.contexts()[0].context_ref,
        second.contexts()[0].context_ref
    );
    assert_ne!(
        first.contexts()[0].context_ref,
        different.contexts()[0].context_ref
    );

    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let input = root.seed(SeedKey::new("input")?, launch_seed(1, "duplicate"))?;
        let context = ChainContext {
            chain_id: 31337,
            network: "local".to_owned(),
        };
        let _first = root.scope().declare_context(context.clone())?;
        let error = root
            .scope()
            .declare_context(context)
            .expect_err("duplicate context ref rejects");
        assert!(matches!(error, PlanError::DuplicateContextRef(_)));
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: input },
        )
    })
    .expect("root still builds after handled duplicate");
}

#[test]
fn state_with_declared_context_emits_context_node_output_and_input_metadata() {
    let mut registry = StateRegistryBuilder::new();
    register_contextual_multiply_state(&mut registry);
    let descriptor = registry
        .snapshot()
        .state_descriptor::<ContextualMultiplyState>()
        .expect("registered state descriptor resolves");
    register_multiply_state(&mut registry);

    let draft = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        registry.snapshot(),
        |root| {
            let input = root.seed(SeedKey::new("input")?, launch_seed(2, "contextual"))?;
            let context = root.scope().declare_context(ChainContext {
                chain_id: 31337,
                network: "local".to_owned(),
            })?;
            let context_bound = root.scope().state::<ContextualMultiplyState, _>(
                StateKey::new("contextual")?,
                &context,
                LaunchConfig { multiplier: 5 },
                input,
            )?;
            let result = root.scope().state::<MultiplyState, _>(
                StateKey::new("consume-contextual")?,
                NoContext,
                LaunchConfig { multiplier: 2 },
                context_bound,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.contexts().len(), 1);
    let context = &draft.contexts()[0];
    match descriptor.context() {
        StateContextDescriptorSpec::Required(requirement) => {
            assert_eq!(
                &context.context_descriptor_id,
                &requirement.context_descriptor_id
            );
            assert_eq!(&context.schema_id, &requirement.schema_id);
            assert_eq!(&context.semantic_type_id, &requirement.semantic_type_id);
            assert_eq!(
                &context.canonicalizer_identity,
                &requirement.canonicalizer_identity
            );
        }
        StateContextDescriptorSpec::NoContext => panic!("contextual state must require context"),
    }

    let context_node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "contextual")
        .expect("contextual node");
    assert_eq!(&context_node.context_descriptor, descriptor.context());
    assert_eq!(
        &context_node.output_context_contract,
        descriptor.output_context()
    );
    assert!(matches!(
        &context_node.context,
        NodeContextSpec::Required { context_ref } if context_ref == &context.context_ref
    ));
    match &context_node.output_context {
        CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => {
            assert_eq!(context_ref, &context.context_ref);
            assert_eq!(resource_kind, &context_resource_kind());
            assert_eq!(stage, &context_stage());
            assert_eq!(
                producer.producer_descriptor_ids.as_slice(),
                std::slice::from_ref(&context_node.state_descriptor_id)
            );
            assert!(!producer.seed_producers_allowed);
        }
        CellContextSpec::NoContext => panic!("contextual output must be bound"),
    }

    let consumer = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "consume-contextual")
        .expect("consumer node");
    match consumer.input.root.as_ref() {
        InputBindingNodeRef::Cell(cell) => {
            assert_eq!(cell.cell_id(), &context_node.output_cell_id);
            match cell.context() {
                InputContextSpec::Required {
                    context_ref,
                    resource_kind,
                    stage,
                    producer,
                } => {
                    assert_eq!(context_ref, &context.context_ref);
                    assert_eq!(resource_kind, &context_resource_kind());
                    assert_eq!(stage, &context_stage());
                    assert_eq!(
                        producer.producer_descriptor_ids.as_slice(),
                        std::slice::from_ref(&context_node.state_descriptor_id)
                    );
                    assert!(!producer.seed_producers_allowed);
                }
                InputContextSpec::NoContext => panic!("consumer input must carry context"),
            }
        }
        other => panic!("unexpected consumer input binding: {other:?}"),
    }
}

#[test]
fn operation_expansion_can_declare_context_and_plan_context_side_effect() {
    let (states, operations) = contextual_mutation_registries();
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("scope key"),
        states,
        operations,
        |root| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let input = root.seed(SeedKey::new("input")?, launch_seed(2, "contextual"))?;
            let result = root.scope().call::<ContextualMutationOperation, _>(
                OperationKey::new("contextual-mutation-operation")?,
                ContextualMutationOperation,
                LaunchConfig { multiplier: 5 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.contexts().len(), 1);
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "contextual-mutation-operation/state")
        .expect("contextual side-effect node");
    assert!(matches!(
        &node.context,
        NodeContextSpec::Required { context_ref } if context_ref == &draft.contexts()[0].context_ref
    ));
    assert!(node.side_effect_resource_claim.is_some());
    assert_eq!(
        node.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    assert!(matches!(
        &node.output_context,
        CellContextSpec::Bound { context_ref, .. } if context_ref == &draft.contexts()[0].context_ref
    ));
}

#[test]
fn linked_compensation_authoring_keeps_remediation_out_of_forward_nodes() {
    let draft = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        compensation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "linked");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let forward_claim = exclusive_resource_claim("mfm.program.test.forward_nonce", 0x61);
            let remediation_claim =
                exclusive_resource_claim("mfm.program.test.remediation_nonce", 0x62);
            let (forward, remediation) = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: forward_claim.clone(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("compensate-forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: remediation_claim.clone(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |forward| Ok(forward.clone()),
                )?;
            assert_ne!(forward.node_id(), remediation.node_id());
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: forward.into_handle(),
                },
            )
        },
    )
    .expect("root builds");

    assert!(matches!(
        draft.saga_policy(),
        SagaPolicy::CompensateCompleted { .. }
    ));
    assert_eq!(draft.state_nodes().len(), 1);
    let forward = &draft.state_nodes()[0];
    assert_eq!(forward.key.as_str(), "forward");
    assert_eq!(forward.runner, RunnerKind::ApplySideEffect);
    let expected_forward_claim = exclusive_resource_claim("mfm.program.test.forward_nonce", 0x61);
    assert_eq!(
        forward.side_effect_resource_claim.as_ref(),
        Some(expected_forward_claim.as_spec())
    );
    assert_eq!(
        forward.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    let verify = forward
        .side_effect_verify
        .as_ref()
        .expect("forward side-effect verify pair");
    assert_ne!(verify.node_id, forward.node_id);
    assert_ne!(verify.output_cell_id, forward.output_cell_id);
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &verify.output_cell_id
    );
    let remediation = draft
        .remediation_nodes()
        .get(&forward.node_id)
        .expect("linked remediation");
    assert_eq!(remediation.key.as_str(), "compensate-forward");
    assert_eq!(remediation.runner, RunnerKind::ApplySideEffect);
    let expected_remediation_claim =
        exclusive_resource_claim("mfm.program.test.remediation_nonce", 0x62);
    assert_eq!(
        remediation.side_effect_resource_claim.as_ref(),
        Some(expected_remediation_claim.as_spec())
    );
    assert_eq!(
        remediation.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    let remediation_verify = remediation
        .side_effect_verify
        .as_ref()
        .expect("remediation side-effect verify pair");
    assert_ne!(remediation_verify.node_id, remediation.node_id);
    assert_ne!(
        remediation_verify.output_cell_id,
        remediation.output_cell_id
    );
    assert!(
        draft
            .state_nodes()
            .iter()
            .all(|node| node.node_id != remediation.node_id),
        "remediation node must not be in forward node collection"
    );
}

#[test]
fn linked_compensation_rejects_same_forward_and_remediation_key() {
    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        compensation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "same-key");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let _ = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |forward| Ok(forward.clone()),
                )?;
            unreachable!("same-key linked compensation should reject");
        },
    )
    .expect_err("same forward/remediation key rejects");

    assert!(matches!(err, PlanError::DuplicateStateKey(key) if key == "forward"));
}

#[test]
fn compensating_policy_requires_every_forward_side_effect_linked() {
    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        forward_mutation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "gap");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let result = root.scope().side_effect::<ForwardMutationState, _>(
                StateKey::new("forward")?,
                NoContext,
                LaunchConfig { multiplier: 5 },
                input,
                manual_resource_claim(),
                SideEffectVerificationSpec::Receipt,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.into_handle(),
                },
            )
        },
    )
    .expect_err("coverage gap rejects");

    assert!(
        matches!(err, PlanError::SagaCoverageGap(message) if message.contains("no linked remediation"))
    );
}

#[test]
fn out_of_scope_remediation_binding_fails_finalize() {
    let mut registry = StateRegistryBuilder::new();
    register_multiply_state(&mut registry);
    register_forward_mutation_state(&mut registry);
    register_compensation_mutation_state(&mut registry);

    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        registry.snapshot(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "scope");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let sibling = root.scope().state::<MultiplyState, _>(
                StateKey::new("sibling")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input.clone(),
            )?;
            let _ = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("compensate-forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |_forward| Ok(sibling),
                )?;
            unreachable!("remediation binding should have rejected");
        },
    )
    .expect_err("out-of-scope remediation input rejects");

    assert!(
        matches!(err, PlanError::RemediationBindingScope(message) if message.contains("outside linked forward"))
    );
}
