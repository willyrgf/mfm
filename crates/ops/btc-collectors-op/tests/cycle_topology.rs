use mfm_op_btc_collectors::{
    btc_collectors_operation_registry, btc_collectors_state_registry,
    BtcAddressBalanceSnapshotFact, BtcNativeBalancesAtAnchorConfig, BtcNetworkCollectionConfig,
    BtcNetworkCollectionOperation, BtcNetworkCollectionReceipt, ObserveBtcAddressBalanceState,
    RecordBtcAddressBalanceFactState, ResolveBtcJointTipState,
};
use mfm_program::{
    build_root_with_registries, InputBindingNodeRef, MfmFactType as _, OperationKey,
    PublicOutputKey, RootBuilder, ScopeKey, StateSpec as _,
};
use mfm_program_derive::PublicOutputs;

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.bitcoin.test.network_collection_public_outputs")]
struct TestNetworkCollectionPublicOutputs<'program, 'scope> {
    receipt: mfm_program::Handle<'program, 'scope, BtcNetworkCollectionReceipt>,
}

fn network_collection_config() -> BtcNetworkCollectionConfig {
    BtcNetworkCollectionConfig {
        native_balances: BtcNativeBalancesAtAnchorConfig {
            network: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            addresses: vec!["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned()],
        },
    }
}

fn network_collection_draft(config: BtcNetworkCollectionConfig) -> mfm_program::TypedProgramDraft {
    build_root_with_registries(
        ScopeKey::new("btc_network_collection_topology").expect("root key"),
        btc_collectors_state_registry().expect("state registry"),
        btc_collectors_operation_registry().expect("operation registry"),
        |root: &mut RootBuilder<'_, '_>| {
            let outputs = root.scope().call::<BtcNetworkCollectionOperation, _>(
                OperationKey::new("network_collection").expect("operation key"),
                BtcNetworkCollectionOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("network_receipt").expect("public output key"),
                &TestNetworkCollectionPublicOutputs {
                    receipt: outputs.receipt,
                },
            )
        },
    )
    .expect("network collection draft")
}

#[test]
fn internal_native_collection_resolves_one_tip_before_all_managed_facts_and_receipt() {
    let first = network_collection_draft(network_collection_config());
    let second = network_collection_draft(network_collection_config());
    assert_eq!(first, second, "network expansion must be deterministic");

    let nodes = first.state_nodes();
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>(),
        [
            "resolve_joint_tip",
            "observe_address_balance_0",
            "record_address_balance_0",
            "assemble_network_collection_receipt",
        ]
    );
    assert_eq!(
        nodes[0].state_kind,
        ResolveBtcJointTipState::kind().unwrap()
    );
    assert_eq!(
        nodes[1].state_kind,
        ObserveBtcAddressBalanceState::kind().unwrap()
    );
    assert_eq!(
        nodes[2].state_kind,
        RecordBtcAddressBalanceFactState::kind().unwrap()
    );

    let tip_cell = nodes[0].output_cell_id.as_str();
    assert_eq!(state_input_cells(&nodes[1], nodes), vec![tip_cell]);
    assert_eq!(
        state_input_cells(&nodes[2], nodes),
        vec![nodes[1].output_cell_id.as_str()]
    );
    let receipt_inputs = state_input_cells(&nodes[3], nodes);
    assert_eq!(receipt_inputs.len(), 2);
    assert!(receipt_inputs.contains(&tip_cell));
    assert!(receipt_inputs.contains(&nodes[2].output_cell_id.as_str()));
}

#[test]
fn descriptor_allow_lists_are_attached_to_fact_recording_nodes_and_survive_certification() {
    let draft = network_collection_draft(network_collection_config());
    let nodes = draft.state_nodes();
    let balance_ref = mfm_program::fact_descriptor_ref::<BtcAddressBalanceSnapshotFact>()
        .expect("balance descriptor ref");
    assert_eq!(
        BtcAddressBalanceSnapshotFact::descriptor()
            .expect("balance descriptor")
            .fact_kind()
            .as_str(),
        "bitcoin.address_balance_snapshot"
    );

    assert!(nodes[0].fact_descriptor_allowlist.is_empty());
    assert!(nodes[1].fact_descriptor_allowlist.is_empty());
    assert_eq!(
        nodes[2].fact_descriptor_allowlist.as_slice(),
        std::slice::from_ref(&balance_ref)
    );
    assert!(nodes[3].fact_descriptor_allowlist.is_empty());

    let certified = mfm_certify::certify_program_draft(&draft).expect("certified");

    certified.envelope().verify_hash().expect("hash verifies");
    let spec = certified.validated_spec().spec();
    let nodes = spec
        .nodes
        .iter()
        .filter(|node| node.framework.is_none())
        .collect::<Vec<_>>();
    assert_eq!(nodes.len(), 4);
    assert!(nodes[0].fact_descriptor_allowlist.is_empty());
    assert!(nodes[1].fact_descriptor_allowlist.is_empty());
    assert_eq!(nodes[2].fact_descriptor_allowlist.len(), 1);
    assert_eq!(nodes[2].fact_descriptor_allowlist[0], balance_ref);
    assert!(nodes[3].fact_descriptor_allowlist.is_empty());
}

#[test]
fn operation_descriptor_registers() {
    let registry = btc_collectors_operation_registry().expect("operation registry");
    let descriptor = registry
        .operation_descriptor::<BtcNetworkCollectionOperation>()
        .expect("operation descriptor");

    assert_eq!(descriptor.name(), "mfm.bitcoin.btc_network_collection");
}

#[test]
fn op_crate_manifest_stays_inside_operation_boundaries() {
    let manifest = include_str!("../Cargo.toml");

    for forbidden in [
        "mfm-app",
        "mfm-runtime",
        "mfm-store",
        "mfm-storage-postgres",
        "mfm-adapters-btc-jsonrpc",
        "mfm-btc-jsonrpc-http",
        "bin/cli",
        "bin/rest-api",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "operation crate must not depend on forbidden boundary crate {forbidden}"
        );
    }
}

fn state_input_cells<'a>(
    node: &'a mfm_program::StateNodeSpec,
    all_nodes: &'a [mfm_program::StateNodeSpec],
) -> Vec<&'a str> {
    let state_output_cells = all_nodes
        .iter()
        .map(|node| node.output_cell_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut cells = Vec::new();
    collect_input_cells(node.input.root.as_ref(), &mut cells);
    cells
        .into_iter()
        .filter(|cell| state_output_cells.contains(cell))
        .collect()
}

fn collect_input_cells<'a>(node: InputBindingNodeRef<'a>, output: &mut Vec<&'a str>) {
    match node {
        InputBindingNodeRef::Unit => {}
        InputBindingNodeRef::Cell(cell) => output.push(cell.cell_id().as_str()),
        InputBindingNodeRef::Tuple(elements) => {
            for element in elements {
                collect_input_cells(element.as_ref(), output);
            }
        }
        InputBindingNodeRef::Struct(fields) => {
            for field in fields {
                collect_input_cells(field.node.as_ref(), output);
            }
        }
        InputBindingNodeRef::Vec { elements, .. }
        | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element.as_ref(), output);
            }
        }
    }
}
