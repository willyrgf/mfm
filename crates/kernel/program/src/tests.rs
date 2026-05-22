use super::*;
use mfm_program_derive::{MfmValue, PublicOutputs as PublicOutputsDerive};
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
