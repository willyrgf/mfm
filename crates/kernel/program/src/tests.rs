use super::*;
use mfm_program_derive::{
    MfmConfig, MfmValue, OperationOutput as OperationOutputDerive,
    PublicOutputs as PublicOutputsDerive, StateInput as StateInputDerive,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.test",
    name = "launch_value",
    version = "1",
    schema = "mfm.program.test.launch_value"
)]
struct LaunchValue {
    amount: u64,
    label: String,
}

#[derive(PublicOutputsDerive)]
#[mfm(schema = "mfm.program.test.public_outputs")]
struct LaunchPublicOutputs<'p, 's> {
    result: Handle<'p, 's, LaunchValue>,
}

#[derive(OperationOutputDerive)]
#[mfm(schema = "mfm.program.test.operation_outputs")]
struct LaunchOperationOutputs<'p, 's> {
    result: Handle<'p, 's, LaunchValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInputDerive)]
#[serde(rename_all = "camelCase")]
struct LaunchInput {
    primary_value: LaunchValue,
    optional_value: mfm_values::MaybeValue<LaunchValue>,
    ordered_values: Vec<LaunchValue>,
    required_values: NonEmpty<LaunchValue>,
    artifact_value: mfm_values::ArtifactRef<LaunchValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[mfm(schema = "mfm.program.test.launch_config")]
struct LaunchConfig {
    multiplier: u64,
}

#[derive(Debug, Clone)]
struct MultiplyState {
    config: LaunchConfig,
}

impl StateSpec for MultiplyState {
    type Config = LaunchConfig;
    type Input = LaunchValue;
    type Output = LaunchValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> Result<StateKind> {
        StateKind::new(
            "mfm.program.test.state",
            "multiply",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.program.test.state:multiply"),
        )
        .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn version() -> Result<StateVersion> {
        StateVersion::new("mfm.program.test.state.multiply.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "multiply"
    }

    fn new(config: Self::Config) -> Result<Self> {
        Ok(Self { config })
    }
}

impl PureState for MultiplyState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(LaunchValue {
            amount: input.amount * self.config.multiplier,
            label: input.label,
        })
    }
}

#[derive(Debug, Clone)]
struct MultiplyOperation;

impl Operation for MultiplyOperation {
    type Config = LaunchConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, LaunchValue>;
    type Output<'program, 'scope> = LaunchOperationOutputs<'program, 'scope>;

    fn kind() -> Result<OperationKind> {
        OperationKind::new(
            "mfm.program.test.operation",
            "multiply",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.program.test.operation:multiply"),
        )
        .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn version() -> Result<OperationVersion> {
        OperationVersion::new("mfm.program.test.operation.multiply.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "multiply"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        input: Self::Input<'program, 'scope>,
        builder: &mut ScopeBuilder<'program, 'scope>,
    ) -> Result<Self::Output<'program, 'scope>> {
        let result = builder.state::<MultiplyState, _>(
            StateKey::new("multiply-operation/state")?,
            config,
            input,
        )?;
        Ok(LaunchOperationOutputs { result })
    }
}

#[test]
fn root_builder_binds_seed_and_public_output_specs() {
    let draft = build_root(
        ScopeKey::new("portfolio/root").expect("scope key"),
        |root| {
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 42,
                label: "cash".to_owned(),
            })?;
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
fn registered_state_registry_plans_state_node() {
    let mut registry = StateRegistryBuilder::new();
    let registered = registry
        .register::<MultiplyState>()
        .expect("state registers");
    assert_eq!(registered.runner(), RunnerKind::Pure);

    let draft = build_root_with_registry(
        ScopeKey::new("portfolio/root").expect("scope key"),
        registry.snapshot(),
        |root| {
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 7,
                label: "gross".to_owned(),
            })?;
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
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

    assert_eq!(draft.state_nodes().len(), 1);
    let node = &draft.state_nodes()[0];
    assert_eq!(node.key.as_str(), "multiply");
    assert_eq!(node.scope_id, *draft.root_scope_id());
    assert_eq!(
        &node.state_descriptor_id,
        registered.descriptor().descriptor_id()
    );
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
fn explicit_registered_state_token_plans_without_builder_registry() {
    let mut registry = StateRegistryBuilder::new();
    let registered = registry
        .register::<MultiplyState>()
        .expect("state registers");

    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 2,
            label: "explicit".to_owned(),
        })?;
        let input = root.seed(SeedKey::new("input")?, seed)?;
        let result = root.scope().state_registered::<MultiplyState, _>(
            StateKey::new("multiply")?,
            registered,
            LaunchConfig { multiplier: 5 },
            input,
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result },
        )
    })
    .expect("root builds");

    assert_eq!(draft.state_nodes().len(), 1);
    assert_eq!(draft.state_nodes()[0].key.as_str(), "multiply");
}

#[test]
fn registered_operation_registry_records_lineage_frame() {
    let mut state_registry = StateRegistryBuilder::new();
    state_registry
        .register::<MultiplyState>()
        .expect("state registers");
    let mut operation_registry = OperationRegistryBuilder::new();
    let registered_operation = operation_registry
        .register::<MultiplyOperation>()
        .expect("operation registers");

    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry.snapshot(),
        operation_registry.snapshot(),
        |root| {
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 4,
                label: "operation".to_owned(),
            })?;
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
        registered_operation.descriptor().descriptor_id()
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
fn explicit_registered_operation_token_calls_without_builder_registry() {
    let mut state_registry = StateRegistryBuilder::new();
    state_registry
        .register::<MultiplyState>()
        .expect("state registers");
    let mut operation_registry = OperationRegistryBuilder::new();
    let registered_operation = operation_registry
        .register::<MultiplyOperation>()
        .expect("operation registers");

    let draft = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        state_registry.into_snapshot(),
        |root| {
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 5,
                label: "explicit-operation".to_owned(),
            })?;
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let result = root.scope().call_registered::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                registered_operation,
                MultiplyOperation,
                LaunchConfig { multiplier: 2 },
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

    assert_eq!(draft.operation_lineage().len(), 1);
    assert_eq!(
        draft.operation_lineage()[0].key.as_str(),
        "multiply-operation"
    );
}

#[test]
fn unregistered_operation_cannot_be_called() {
    let mut state_registry = StateRegistryBuilder::new();
    state_registry
        .register::<MultiplyState>()
        .expect("state registers");

    let result = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        state_registry.into_snapshot(),
        |root| {
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 1,
                label: "unregistered-operation".to_owned(),
            })?;
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let _ = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 2 },
                input,
            )?;
            unreachable!("unregistered operation planning must fail before public output binding")
        },
    );

    let Err(PlanError::Registry(message)) = result else {
        panic!("expected unregistered operation registry error, got {result:?}");
    };
    assert!(message.contains("operation"));
    assert!(message.contains("is not registered"));
}

#[test]
fn unregistered_state_cannot_be_planned() {
    let result = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 1,
            label: "unregistered".to_owned(),
        })?;
        let input = root.seed(SeedKey::new("input")?, seed)?;
        let _ = root.scope().state::<MultiplyState, _>(
            StateKey::new("multiply")?,
            LaunchConfig { multiplier: 2 },
            input,
        )?;
        unreachable!("unregistered state planning must fail before public output binding")
    });

    let Err(PlanError::Registry(message)) = result else {
        panic!("expected unregistered state registry error, got {result:?}");
    };
    assert!(message.contains("is not registered"));
}

#[test]
fn duplicate_state_key_is_rejected_without_partial_node() {
    let mut registry = StateRegistryBuilder::new();
    registry
        .register::<MultiplyState>()
        .expect("state registers");

    let result = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        registry.into_snapshot(),
        |root| {
            let first = root.seed(
                SeedKey::new("first")?,
                CanonicalSeed::from_value(&LaunchValue {
                    amount: 1,
                    label: "first".to_owned(),
                })?,
            )?;
            let second = root.seed(
                SeedKey::new("second")?,
                CanonicalSeed::from_value(&LaunchValue {
                    amount: 2,
                    label: "second".to_owned(),
                })?,
            )?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                LaunchConfig { multiplier: 2 },
                first,
            )?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
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

#[test]
fn single_handle_input_binding_records_identity_and_lineage() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 10,
            label: "single".to_owned(),
        })?;
        let handle = root.seed(SeedKey::new("single")?, seed)?;
        let binding: InputBinding<LaunchValue> = handle.clone().into_binding()?;

        let InputBindingNodeKind::Cell(cell) = &binding.root().kind else {
            panic!("single handle should bind as a cell");
        };

        assert_eq!(cell.field_path.as_str(), "");
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

        let final_seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 11,
            label: "terminal".to_owned(),
        })?;
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
        let first = root.seed(
            SeedKey::new("first")?,
            CanonicalSeed::from_value(&LaunchValue {
                amount: 1,
                label: "first".to_owned(),
            })?,
        )?;
        let second = root.seed(
            SeedKey::new("second")?,
            CanonicalSeed::from_value(&LaunchValue {
                amount: 2,
                label: "second".to_owned(),
            })?,
        )?;

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
        let InputBindingNodeKind::Vec { elements, ordering } = &vector_binding.root().kind else {
            panic!("vector input should bind as vector");
        };
        assert_eq!(*ordering, OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(cell_path(elements.first().expect("first vector cell")), "0");
        assert_eq!(cell_path(elements.get(1).expect("second vector cell")), "1");

        assert_eq!(
            NonEmptyHandles::<LaunchValue>::try_from_vec(Vec::new()).expect_err("empty rejects"),
            PlanError::EmptyNonEmptyInput
        );
        let non_empty_binding: InputBinding<NonEmpty<LaunchValue>> =
            NonEmptyHandles::try_from_vec(vec![first.clone(), second.clone()])?.into_binding()?;
        let InputBindingNodeKind::NonEmptyVec { elements, ordering } =
            &non_empty_binding.root().kind
        else {
            panic!("non-empty input should bind as non-empty vector");
        };
        assert_eq!(*ordering, OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(elements.len(), 2);

        let empty_node =
            InputBindingNode::non_empty_vector(Vec::new(), OrderingEvidence::ExplicitAuthorOrder);
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
        let primary = root.seed(
            SeedKey::new("primary")?,
            CanonicalSeed::from_value(&LaunchValue {
                amount: 1,
                label: "primary".to_owned(),
            })?,
        )?;
        let optional = root.seed(
            SeedKey::new("optional")?,
            CanonicalSeed::from_value(&mfm_values::MaybeValue::Produced(LaunchValue {
                amount: 2,
                label: "optional".to_owned(),
            }))?,
        )?;
        let ordered_first = root.seed(
            SeedKey::new("ordered-first")?,
            CanonicalSeed::from_value(&LaunchValue {
                amount: 3,
                label: "ordered-first".to_owned(),
            })?,
        )?;
        let ordered_second = root.seed(
            SeedKey::new("ordered-second")?,
            CanonicalSeed::from_value(&LaunchValue {
                amount: 4,
                label: "ordered-second".to_owned(),
            })?,
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
        let InputBindingNodeKind::Vec { elements, ordering } = &ordered_field.node.kind else {
            panic!("orderedValues should bind as a vector");
        };
        assert_eq!(*ordering, OrderingEvidence::ExplicitAuthorOrder);
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
            let seed = CanonicalSeed::from_value(&LaunchValue {
                amount: 42,
                label: "cash".to_owned(),
            })?;
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
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 42,
            label: "cash".to_owned(),
        })?;
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
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 1,
            label: "left".to_owned(),
        })?;
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
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 2,
            label: "right".to_owned(),
        })?;
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

#[test]
fn duplicate_seed_keys_reject() {
    let error = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = CanonicalSeed::from_value(&LaunchValue {
            amount: 1,
            label: "a".to_owned(),
        })?;
        let second = CanonicalSeed::from_value(&LaunchValue {
            amount: 2,
            label: "b".to_owned(),
        })?;
        let _ = root.seed(SeedKey::new("same")?, first)?;
        let _ = root.seed(SeedKey::new("same")?, second)?;
        unreachable!("duplicate seed must reject before binding outputs")
    })
    .expect_err("duplicate seed rejects");

    assert_eq!(error, PlanError::DuplicateSeedKey("same".to_owned()));
}

#[test]
fn keys_reject_non_ascii_and_empty_values() {
    assert!(ScopeKey::new("").is_err());
    assert!(ScopeKey::new("Root").is_err());
    assert!(SeedKey::new("semente-á").is_err());
    assert!(PublicFieldPath::new("result.total").is_ok());
}

#[test]
fn canonical_seed_can_be_built_from_canonical_json() {
    let bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        "{\"amount\":7,\"label\":\"canonical\"}",
    )
    .expect("canonical json");
    let seed = CanonicalSeed::<LaunchValue>::from_canonical_json(bytes).expect("seed");

    assert_eq!(seed.byte_len(), 32);
    assert_eq!(
        seed.content_digest().as_str(),
        "content:sha256-jcs-v1:2b2d92093ac043c94672798bbc5c79761eec80b4aed995400305e8f8a06927e2"
    );
    assert_eq!(
        seed.canonical_json().as_str(),
        "{\"amount\":7,\"label\":\"canonical\"}"
    );
}

#[test]
fn canonical_seed_rejects_json_that_does_not_decode_as_value_type() {
    let bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{\"amount\":7}")
        .expect("canonical json");
    let error =
        CanonicalSeed::<LaunchValue>::from_canonical_json(bytes).expect_err("missing label");

    assert!(matches!(error, PlanError::Canonical(message) if message.contains("missing field")));
}

#[test]
fn root_scope_id_is_stable_for_key() {
    let left = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 1,
            label: "a".to_owned(),
        })?;
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("left");
    let right = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = CanonicalSeed::from_value(&LaunchValue {
            amount: 2,
            label: "b".to_owned(),
        })?;
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("right");

    assert_eq!(left.root_scope_id(), right.root_scope_id());
}

fn cell_path(node: &InputBindingNode) -> &str {
    let InputBindingNodeKind::Cell(cell) = &node.kind else {
        panic!("expected cell node");
    };
    cell.field_path.as_str()
}

fn artifact_ref_fixture(byte: u8) -> Result<mfm_values::ArtifactRef<LaunchValue>> {
    let artifact_id = mfm_ids::ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    );
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte.wrapping_add(1); 32]),
    );
    mfm_values::ArtifactRef::<LaunchValue>::new(artifact_id, content_digest)
        .map_err(|error| PlanError::Value(error.to_string()))
}
