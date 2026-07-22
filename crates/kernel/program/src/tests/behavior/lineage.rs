use super::*;

#[test]
fn state_output_domain_keys_are_lineage_evidence() {
    let registry = multiply_state_registry();

    let build = |domain_key: TestDomainKey| {
        build_root_with_registry(
            ScopeKey::new("root").expect("scope key"),
            registry.clone(),
            |root| {
                let seed = launch_seed(2, "domain");
                let input = root.seed(SeedKey::new("input")?, seed)?;
                let result = root.scope().state_with_domain_keys::<MultiplyState, _, _>(
                    StateKey::new("multiply")?,
                    NoContext,
                    LaunchConfig { multiplier: 5 },
                    input,
                    vec![domain_key],
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &LaunchPublicOutputs { result },
                )
            },
        )
        .expect("root builds")
    };

    let first = build(TestDomainKey {
        source: "portfolio".to_owned(),
        index: 1,
    });
    let second = build(TestDomainKey {
        source: "portfolio".to_owned(),
        index: 2,
    });

    let first_node = &first.state_nodes()[0];
    let second_node = &second.state_nodes()[0];
    assert_eq!(first_node.output_domain_keys.len(), 1);
    assert_eq!(first_node.node_id, second_node.node_id);
    assert_eq!(first_node.output_cell_id, second_node.output_cell_id);
    assert_ne!(
        first_node.output_value_lineage, second_node.output_value_lineage,
        "stable domain keys must be hash-defining for state output lineage"
    );
}

#[test]
fn registered_operation_registry_records_lineage_frame() {
    let state_registry = multiply_state_registry();
    let mut operation_registry = OperationRegistryBuilder::new();
    register_multiply_operation(&mut operation_registry);
    let operation_descriptor = operation_registry
        .snapshot()
        .operation_descriptor::<MultiplyOperation>()
        .expect("registered operation descriptor resolves");

    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry.snapshot(),
        |root| {
            let seed = launch_seed(4, "operation");
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

    assert_eq!(draft.state_nodes().len(), 1);
    assert_eq!(draft.operation_lineage().len(), 1);
    let frame = &draft.operation_lineage()[0];
    assert_eq!(frame.key.as_str(), "multiply-operation");
    assert_eq!(frame.scope_id, *draft.root_scope_id());
    assert_eq!(
        &frame.operation_descriptor_id,
        operation_descriptor.descriptor_id()
    );
    assert_eq!(
        frame.output_schema_id,
        <LaunchOperationOutputs<'static, 'static> as OperationOutput<'static, 'static>>::output_schema_id()
            .expect("output schema id")
    );
    assert_eq!(frame.output_handles.len(), 1);
    assert_eq!(
        frame.output_handles[0].cell_id(),
        &draft.state_nodes()[0].output_cell_id
    );
    assert!(frame
        .lineage_digest
        .as_str()
        .starts_with("content:sha256-jcs-v1:"));
}

#[test]
fn stable_ids_and_value_lineage_golden_vectors() {
    let (state_registry, operation_registry) = multiply_registries();

    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry,
        |root| {
            let seed = launch_seed(4, "operation");
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

    let seed = &draft.seeds()[0];
    let node = &draft.state_nodes()[0];
    let frame = &draft.operation_lineage()[0];
    let empty_lineage = OperationLineage::empty().expect("empty lineage");
    let active_lineage =
        OperationLineage::from_parts(vec![frame.operation_instance_id.clone()], Vec::new())
            .expect("active lineage");
    let unsupported_lowering_node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": node.config.config_ref_digest.as_str(),
            "input_binding_digest": node.input.digest.as_str(),
            "local_node_key": node.key.as_str(),
            "lowering_version": "mfm.typed.lowering.unsupported",
            "scope_id": node.scope_id.as_str(),
            "state_kind": node.state_kind.as_str(),
            "state_version": node.state_version.as_str(),
        }))
        .expect("alternate lowering digest"),
    );

    assert_eq!(
        draft.root_scope_id().as_str(),
        "scope:sha256-jcs-v1:b773e3f182b4866afd06c3948883ad1545bd26152fa9ebc9a9df4f67ae7151b9"
    );
    assert_eq!(
        empty_lineage.digest.as_str(),
        "content:sha256-jcs-v1:c593e42bace983cf3dd7de37223c5f442e305ec8e5a37e828d23c4a9e8584f22"
    );
    assert_eq!(
        seed.seed_id.as_str(),
        "seed:sha256-jcs-v1:f13eb7cea8383588ea4b87607db8074478c8212ee4ce3660220224fd0636d745"
    );
    assert_eq!(
        seed.cell_id.as_str(),
        "cell:sha256-jcs-v1:86089606d76c84a3924bda1ab24ae860ef0d66c7bf14451ac064c4fc1f19760e"
    );
    assert_eq!(
        seed.value_lineage.digest().as_str(),
        "content:sha256-jcs-v1:f35696d73ebd308f69af9d2b4b42a016759cea2e69310fdbec2dbe52edb7bb4e"
    );
    assert_eq!(
        frame.config.config_ref_digest.as_str(),
        "content:sha256-jcs-v1:4efd60dfc94f22725016efa4bef18e7571683bb788d685c943b1e44266575555"
    );
    assert_eq!(
        node.config.config_ref_digest,
        frame.config.config_ref_digest
    );
    assert_eq!(
        frame.input.digest.as_str(),
        "content:sha256-jcs-v1:3f5608a25f7712ff8d66c29994c1b47328d38abd4b62de0d7236a894b92d262e"
    );
    assert_eq!(node.input.digest, frame.input.digest);
    assert_eq!(
        frame.operation_instance_id.as_str(),
        "op:sha256-jcs-v1:f25fb307efaa3c12d527f43be02ba4da49dd081e01ac9628c20d0d350b8f01e0"
    );
    assert_eq!(
        active_lineage.digest.as_str(),
        "content:sha256-jcs-v1:34ae2a84e1d8a901d440e9adbf10906b272f48a11f0e40c32224111c81d1ca66"
    );
    assert_eq!(
        node.node_id.as_str(),
        "node:sha256-jcs-v1:f98d61dd285dcde234773229c270e687ff78e0c384b73ab3c60a0f68d10f756b"
    );
    assert_ne!(
        node.node_id, unsupported_lowering_node_id,
        "lowering version must be hash-defining for node ids"
    );
    assert_eq!(
        node.output_cell_id.as_str(),
        "cell:sha256-jcs-v1:1578a57b9a02573ca41721d1352cf74205085a4cb3fda5e08450d6ba99386738"
    );
    assert_eq!(
        node.output_value_lineage.digest().as_str(),
        "content:sha256-jcs-v1:47fc3f4a054bc9a0ddc217f9317d3ff049d29275395177b2973ddcac7a8d9276"
    );
    assert_eq!(
        frame.lineage_digest.as_str(),
        "content:sha256-jcs-v1:99fd32cf325b6b69ca0754e20b3ff217028b7e32103d7ca4c864a0353e668ea7"
    );
}

#[test]
fn domain_keyed_handles_sort_by_stable_refs_and_reject_duplicates() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let late = root.seed(SeedKey::new("late")?, launch_seed(3, "late"))?;
        let early = root.seed(SeedKey::new("early")?, launch_seed(1, "early"))?;
        let late_ref = late.typed_ref();
        let early_ref = early.typed_ref();
        let keyed = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ])?;
        let reversed_keyed = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
        ])?;
        let binding: InputBinding<Vec<LaunchValue>> = keyed.into_binding()?;
        let reversed_binding: InputBinding<Vec<LaunchValue>> = reversed_keyed.into_binding()?;
        assert_eq!(
            binding.digest(),
            reversed_binding.digest(),
            "stable domain-key ref order must make input order irrelevant"
        );
        let InputBindingNodeKind::Vec {
            ordering,
            domain_keys,
            elements,
        } = &binding.root().kind
        else {
            panic!("domain-keyed handles should bind as vector");
        };
        assert_eq!(ordering, &OrderingEvidence::StableDomainKey);
        assert_eq!(domain_keys.len(), 2);
        assert_eq!(elements.len(), 2);
        assert!(domain_keys.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(
            domain_keys[0].content_digest.as_str(),
            "content:sha256-jcs-v1:201a76ed8dddaee1cc860ade5d752a9b6e710be9f1ccece57a7c69f6a062e64f"
        );
        assert_eq!(
            domain_keys[1].content_digest.as_str(),
            "content:sha256-jcs-v1:dcf7ba6724b36bcbb4945ba25c50e8fadc7b56b00be10009321a918484de9187"
        );
        let InputBindingNodeKind::Cell(first_cell) = &elements[0].kind else {
            panic!("first domain-keyed element should be a cell");
        };
        let InputBindingNodeKind::Cell(second_cell) = &elements[1].kind else {
            panic!("second domain-keyed element should be a cell");
        };
        assert_eq!(first_cell.cell_id, late_ref.cell_id);
        assert_eq!(second_cell.cell_id, early_ref.cell_id);

        let non_empty_keyed = DomainKeyedNonEmptyHandles::new(vec![
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ])?;
        let non_empty_binding: InputBinding<NonEmpty<LaunchValue>> =
            non_empty_keyed.into_binding()?;
        let InputBindingNodeKind::NonEmptyVec {
            ordering,
            domain_keys,
            elements,
        } = &non_empty_binding.root().kind
        else {
            panic!("domain-keyed non-empty handles should bind as non-empty vector");
        };
        assert_eq!(ordering, &OrderingEvidence::StableDomainKey);
        assert_eq!(domain_keys.len(), 2);
        assert_eq!(elements.len(), 2);
        assert!(domain_keys.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(matches!(
            DomainKeyedNonEmptyHandles::<TestDomainKey, LaunchValue>::new(Vec::new()),
            Err(PlanError::EmptyNonEmptyInput)
        ));

        let duplicate = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "dup".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
            (
                TestDomainKey {
                    source: "dup".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ]);
        assert!(matches!(duplicate, Err(PlanError::DuplicateDomainKey(_))));

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: early },
        )
    })
    .expect("root builds");
}

#[test]
fn same_scope_same_type_lineage_mismatch_rejects_for_certification() {
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
        let _second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");

    let first = draft.seeds()[0].typed_ref();
    let second = draft.seeds()[1].typed_ref();
    assert!(matches!(
        draft.validate_same_scope_same_type_lineage_for_certification(&first, &second),
        Err(PlanError::LineageMismatch(_))
    ));
    assert!(draft
        .validate_same_scope_same_type_lineage_for_certification(&first, &first)
        .is_ok());
}

#[test]
fn failed_operation_expansion_rolls_back_scope_mutations_and_lineage() {
    let state_registry = multiply_state_registry();
    let mut operation_registry = OperationRegistryBuilder::new();
    operation_registry
        .register::<FailingOperation>()
        .expect("failing operation registers");
    register_multiply_operation(&mut operation_registry);

    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("scope key"),
        state_registry,
        operation_registry.into_snapshot(),
        |root| {
            let seed = launch_seed(5, "rollback");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let failed = root.scope().call::<FailingOperation, _>(
                OperationKey::new("rollback-operation")?,
                FailingOperation,
                LaunchConfig { multiplier: 2 },
                input.clone(),
            );
            let Err(error) = failed else {
                panic!("expansion should fail before returning outputs");
            };
            assert_eq!(error, PlanError::Key("forced expansion failure".to_owned()));

            let _direct = root.scope().state::<MultiplyState, _>(
                StateKey::new("rollback/state")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input.clone(),
            )?;
            root.scope()
                .child_scope(ScopeKey::new("rollback/child")?, |child| {
                    child.bridge_to_parent(())
                })?;
            let retry = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("rollback-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 4 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: retry.result,
                },
            )
        },
    )
    .expect("root builds after failed expansion rollback");

    assert_eq!(draft.operation_lineage().len(), 1);
    assert_eq!(
        draft.operation_lineage()[0].key.as_str(),
        "rollback-operation"
    );
    assert_eq!(
        draft
            .scopes()
            .iter()
            .filter(|scope| scope.key.as_str() == "rollback/child")
            .count(),
        1
    );
    assert_eq!(
        draft
            .state_nodes()
            .iter()
            .filter(|node| node.key.as_str() == "rollback/state")
            .count(),
        1
    );
    let direct_state = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "rollback/state")
        .expect("direct state reused failed expansion state key");
    assert!(direct_state.planning_lineage.active_instances.is_empty());
    assert!(direct_state.planning_lineage.completed_frames.is_empty());
}

#[test]
fn unregistered_operation_and_state_are_rejected_at_registry_boundary() {
    enum Case {
        Operation,
        State,
    }

    for case in [Case::Operation, Case::State] {
        let result = match case {
            Case::Operation => build_root_with_registry(
                ScopeKey::new("root").expect("scope key"),
                multiply_state_registry(),
                |root| {
                    let seed = launch_seed(1, "unregistered-operation");
                    let input = root.seed(SeedKey::new("input")?, seed)?;
                    let _ = root.scope().call::<MultiplyOperation, _>(
                        OperationKey::new("multiply-operation")?,
                        MultiplyOperation,
                        LaunchConfig { multiplier: 2 },
                        input,
                    )?;
                    unreachable!(
                        "unregistered operation planning must fail before public output binding"
                    )
                },
            ),
            Case::State => build_root(ScopeKey::new("root").expect("scope key"), |root| {
                let seed = launch_seed(1, "unregistered");
                let input = root.seed(SeedKey::new("input")?, seed)?;
                let _ = root.scope().state::<MultiplyState, _>(
                    StateKey::new("multiply")?,
                    NoContext,
                    LaunchConfig { multiplier: 2 },
                    input,
                )?;
                unreachable!("unregistered state planning must fail before public output binding")
            }),
        };

        let Err(PlanError::Registry(message)) = result else {
            panic!("expected unregistered registry error, got {result:?}");
        };
        if matches!(case, Case::Operation) {
            assert!(message.contains("operation"));
        }
        assert!(message.contains("is not registered"));
    }
}

#[test]
fn duplicate_state_key_is_rejected_without_partial_node() {
    let result = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        multiply_state_registry(),
        |root| {
            let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
            let second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 2 },
                first,
            )?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                second,
            )?;
            unreachable!("duplicate state key must fail")
        },
    );

    assert_eq!(
        result.expect_err("duplicate key rejects"),
        PlanError::DuplicateStateKey("multiply".to_owned())
    );
}
