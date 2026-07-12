use super::*;

#[test]
fn single_handle_input_binding_records_identity_and_lineage() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = launch_seed(10, "single");
        let handle = root.seed(SeedKey::new("single")?, seed)?;
        let binding: InputBinding<LaunchValue> = handle.clone().into_binding()?;

        let InputBindingNodeKind::Cell(cell) = &binding.root().kind else {
            panic!("single handle should bind as a cell");
        };

        assert_eq!(cell.field_path.as_str(), "root");
        assert_eq!(cell.cell_id, handle.typed_ref().cell_id);
        assert_eq!(
            cell.semantic_type_id,
            LaunchValue::semantic_id().expect("semantic id")
        );
        assert_eq!(cell.schema_id, LaunchValue::schema_id().expect("schema id"));
        assert_eq!(cell.required_terminal, RequiredTerminal::ProducedOnly);
        assert_eq!(&cell.value_lineage, handle.typed_ref().value_lineage());
        assert!(binding
            .digest()
            .as_str()
            .starts_with("content:sha256-jcs-v1:"));

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: handle },
        )
    })
    .expect("root builds");
}

#[test]
fn optional_and_artifact_handles_bind_as_typed_value_cells() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let optional_seed =
            CanonicalSeed::from_value(&mfm_values::MaybeValue::Skipped(mfm_values::SkipReason {
                code: mfm_values::SkipCode::Filtered,
                explanation: "filtered by test fixture".to_owned(),
            }))?;
        let optional = root.seed(SeedKey::new("optional")?, optional_seed)?;
        let optional_binding: InputBinding<mfm_values::MaybeValue<LaunchValue>> =
            optional.into_binding()?;

        let InputBindingNodeKind::Cell(cell) = &optional_binding.root().kind else {
            panic!("optional handle should bind as a cell");
        };
        assert_eq!(cell.required_terminal, RequiredTerminal::MaybeSkipped);
        assert_eq!(
            cell.semantic_type_id,
            mfm_values::MaybeValue::<LaunchValue>::semantic_id().expect("maybe semantic id")
        );

        let artifact_ref = artifact_ref_fixture(0xaa)?;
        let artifact = root.seed(
            SeedKey::new("artifact")?,
            CanonicalSeed::from_value(&artifact_ref)?,
        )?;
        let artifact_binding: InputBinding<mfm_values::ArtifactRef<LaunchValue>> =
            artifact.into_binding()?;
        let InputBindingNodeKind::Cell(cell) = &artifact_binding.root().kind else {
            panic!("artifact handle should bind as a cell");
        };
        assert_eq!(cell.required_terminal, RequiredTerminal::ProducedOnly);
        assert_eq!(
            cell.semantic_type_id,
            mfm_values::ArtifactRef::<LaunchValue>::semantic_id().expect("artifact semantic id")
        );

        let final_seed = launch_seed(11, "terminal");
        let final_handle = root.seed(SeedKey::new("terminal-seed")?, final_seed)?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs {
                result: final_handle,
            },
        )
    })
    .expect("root builds");
}

#[test]
fn tuple_vector_and_non_empty_inputs_preserve_author_order() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
        let second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;

        let tuple_binding: InputBinding<(LaunchValue, LaunchValue)> =
            (first.clone(), second.clone()).into_binding()?;
        let InputBindingNodeKind::Tuple { elements } = &tuple_binding.root().kind else {
            panic!("tuple input should bind as tuple");
        };
        assert_eq!(elements.len(), 2);
        assert_eq!(cell_path(elements.first().expect("first tuple cell")), "0");
        assert_eq!(cell_path(elements.get(1).expect("second tuple cell")), "1");

        let vector_binding: InputBinding<Vec<LaunchValue>> =
            vec![first.clone(), second.clone()].into_binding()?;
        let reversed_binding: InputBinding<Vec<LaunchValue>> =
            vec![second.clone(), first.clone()].into_binding()?;
        assert_ne!(
            vector_binding.digest(),
            reversed_binding.digest(),
            "explicit vector order is part of the binding digest"
        );
        let InputBindingNodeKind::Vec {
            elements, ordering, ..
        } = &vector_binding.root().kind
        else {
            panic!("vector input should bind as vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(cell_path(elements.first().expect("first vector cell")), "0");
        assert_eq!(cell_path(elements.get(1).expect("second vector cell")), "1");

        assert_eq!(
            NonEmptyHandles::<LaunchValue>::try_from_vec(Vec::new()).expect_err("empty rejects"),
            PlanError::EmptyNonEmptyInput
        );
        let non_empty_binding: InputBinding<NonEmpty<LaunchValue>> =
            NonEmptyHandles::try_from_vec(vec![first.clone(), second.clone()])?.into_binding()?;
        let InputBindingNodeKind::NonEmptyVec {
            elements, ordering, ..
        } = &non_empty_binding.root().kind
        else {
            panic!("non-empty input should bind as non-empty vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(elements.len(), 2);

        let empty_node = InputBindingNode::non_empty_vector(
            Vec::new(),
            OrderingEvidence::ExplicitAuthorOrder,
            Vec::new(),
        );
        assert_eq!(
            InputBinding::<NonEmpty<LaunchValue>>::from_root(empty_node)
                .expect_err("public empty NonEmptyVec rejects"),
            PlanError::EmptyNonEmptyInput
        );

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");
}

#[test]
fn derive_backed_state_input_handles_build_canonical_struct_bindings() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let primary = root.seed(SeedKey::new("primary")?, launch_seed(1, "primary"))?;
        let optional = root.seed(
            SeedKey::new("optional")?,
            CanonicalSeed::from_value(&mfm_values::MaybeValue::Produced(LaunchValue {
                amount: 2,
                label: "optional".to_owned(),
            }))?,
        )?;
        let ordered_first = root.seed(
            SeedKey::new("ordered-first")?,
            launch_seed(3, "ordered-first"),
        )?;
        let ordered_second = root.seed(
            SeedKey::new("ordered-second")?,
            launch_seed(4, "ordered-second"),
        )?;
        let artifact = root.seed(
            SeedKey::new("artifact")?,
            CanonicalSeed::from_value(&artifact_ref_fixture(0xbb)?)?,
        )?;

        let handles = LaunchInputHandles {
            primary_value: primary.clone(),
            optional_value: optional,
            ordered_values: vec![ordered_first.clone(), ordered_second.clone()],
            required_values: NonEmptyHandles::new(ordered_first, vec![ordered_second]),
            artifact_value: artifact,
        };
        let binding: InputBinding<LaunchInput> = handles.into_binding()?;
        assert_eq!(
            binding.input_schema_id(),
            &LaunchInput::input_schema_id().map_err(|error| PlanError::Value(error.to_string()))?
        );

        let InputBindingNodeKind::Struct { fields } = &binding.root().kind else {
            panic!("derive-backed state input should bind as a struct");
        };
        let paths = fields
            .iter()
            .map(|field| field.field_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "artifactValue",
                "optionalValue",
                "orderedValues",
                "primaryValue",
                "requiredValues",
            ]
        );
        let ordered_field = fields
            .iter()
            .find(|field| field.field_path.as_str() == "orderedValues")
            .expect("ordered field");
        let InputBindingNodeKind::Vec {
            elements, ordering, ..
        } = &ordered_field.node.kind
        else {
            panic!("orderedValues should bind as a vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(
            cell_path(elements.first().expect("first ordered cell")),
            "orderedValues.0"
        );

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: primary },
        )
    })
    .expect("root builds");
}

#[test]
fn duplicate_input_field_paths_reject() {
    let first = NamedInputBinding::new(
        InputFieldPath::new("duplicate").expect("path"),
        InputBindingNode::Unit,
    );
    let second = NamedInputBinding::new(
        InputFieldPath::new("duplicate").expect("path"),
        InputBindingNode::Unit,
    );

    assert_eq!(
        InputBindingNode::struct_fields(vec![first.clone(), second.clone()])
            .expect_err("constructor rejects duplicate paths"),
        PlanError::DuplicateInputFieldPath("duplicate".to_owned())
    );
    assert_eq!(
        InputBinding::<()>::from_root(InputBindingNode::struct_fields_unchecked(vec![
            first, second,
        ]))
        .expect_err("public struct node rejects duplicate paths"),
        PlanError::DuplicateInputFieldPath("duplicate".to_owned())
    );
}

#[test]
fn input_binding_root_must_match_declared_state_input_shape() {
    let error = InputBinding::<LaunchValue>::from_root(InputBindingNode::Unit)
        .expect_err("non-unit input cannot use unit binding root");

    assert!(matches!(
        error,
        PlanError::InputBindingShape(message)
            if message.contains("expected value_ref but got unit")
    ));
}

#[test]
fn child_scope_exports_bridge_nodes_and_validates_refs() {
    let draft = build_root(
        ScopeKey::new("portfolio/root").expect("scope key"),
        |root| {
            let seed = launch_seed(42, "cash");
            let parent_handle = root.seed(SeedKey::new("launch-input")?, seed)?;
            let child_output = root
                .scope()
                .child_scope(ScopeKey::new("execution")?, |child| {
                    let child_handle = child.import_from_parent(
                        BridgeKey::new("import-launch")?,
                        parent_handle.clone(),
                        BridgePolicy::same_run_same_value(),
                    )?;
                    let parent_output = child.export_to_parent(
                        BridgeKey::new("export-result")?,
                        child_handle,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(parent_output)
                })?;
            let outputs = LaunchPublicOutputs {
                result: child_output,
            };
            root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .expect("root builds");

    assert_eq!(draft.scopes().len(), 2);
    assert_eq!(draft.scopes()[1].key.as_str(), "execution");
    assert_eq!(
        draft.scopes()[1].parent_scope_id.as_ref(),
        Some(draft.root_scope_id())
    );
    assert_eq!(draft.bridge_nodes().len(), 2);
    assert_eq!(
        draft.bridge_nodes()[0].bridge_kind,
        BridgeKind::ImportFromParent
    );
    assert_eq!(
        draft.bridge_nodes()[1].bridge_kind,
        BridgeKind::ExportToParent
    );
    assert_eq!(
        draft.bridge_nodes()[0].source_scope_id,
        *draft.root_scope_id()
    );
    assert_eq!(
        draft.bridge_nodes()[0].target_scope_id,
        draft.scopes()[1].scope_id
    );
    assert_eq!(
        draft.bridge_nodes()[1].source_scope_id,
        draft.scopes()[1].scope_id
    );
    assert_eq!(
        draft.bridge_nodes()[1].target_scope_id,
        *draft.root_scope_id()
    );

    let export_ref = draft.bridge_nodes()[1].bridge_ref();
    let certified = draft
        .validate_bridge_ref_for_certification(&export_ref)
        .expect("emitted bridge ref certifies");
    assert_eq!(certified.node_id, export_ref.bridge_node_id);
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &export_ref.target_cell_id
    );
}

#[test]
fn already_parent_bridged_handle_can_flow_through_later_child_scope() {
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = launch_seed(42, "cash");
        let parent_handle = root.seed(SeedKey::new("launch-input")?, seed)?;
        let first_output = root.scope().child_scope(ScopeKey::new("first")?, |child| {
            let child_handle = child.import_from_parent(
                BridgeKey::new("import-launch")?,
                parent_handle.clone(),
                BridgePolicy::same_run_same_value(),
            )?;
            let parent_output = child.export_to_parent(
                BridgeKey::new("export-result")?,
                child_handle,
                BridgePolicy::same_run_same_value(),
            )?;
            child.bridge_to_parent(parent_output)
        })?;
        let second_output = root
            .scope()
            .child_scope(ScopeKey::new("second")?, |child| {
                child.bridge_to_parent(first_output.clone())
            })?;
        let outputs = LaunchPublicOutputs {
            result: second_output,
        };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("root builds");

    assert_eq!(draft.scopes().len(), 3);
    assert_eq!(
        draft.bridge_nodes().len(),
        2,
        "returning an already parent-visible handle must not invent a bridge"
    );
}

#[test]
fn forged_or_stale_bridge_refs_do_not_certify() {
    let left = build_root(ScopeKey::new("left").expect("scope key"), |root| {
        let seed = launch_seed(1, "left");
        let parent_handle = root.seed(SeedKey::new("input")?, seed)?;
        let child_output = root.scope().child_scope(ScopeKey::new("child")?, |child| {
            let child_handle = child.import_from_parent(
                BridgeKey::new("import")?,
                parent_handle.clone(),
                BridgePolicy::same_run_same_value(),
            )?;
            let exported = child.export_to_parent(
                BridgeKey::new("export")?,
                child_handle,
                BridgePolicy::same_run_same_value(),
            )?;
            child.bridge_to_parent(exported)
        })?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs {
                result: child_output,
            },
        )
    })
    .expect("left builds");
    let right = build_root(ScopeKey::new("right").expect("scope key"), |root| {
        let seed = launch_seed(2, "right");
        let parent_handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs {
            result: parent_handle,
        };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("right builds");

    let stale_ref = left.bridge_nodes()[1].bridge_ref();
    assert_eq!(
        right.validate_bridge_ref_for_certification(&stale_ref),
        Err(PlanError::UnknownBridgeRef)
    );

    let mut forged_ref = stale_ref;
    forged_ref.target_cell_id = right.public_output_spec().outputs()[0]
        .cell()
        .cell_id()
        .clone();
    assert_eq!(
        left.validate_bridge_ref_for_certification(&forged_ref),
        Err(PlanError::UnknownBridgeRef)
    );
}
