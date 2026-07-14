use mfm_op_btc_collectors::{
    btc_chain_head_collector_cycle_program_draft, btc_collectors_operation_registry,
    BtcChainHeadCollectorConfig, BtcChainHeadCollectorCycleOperation, BtcChainHeadFact,
    CollectorCheckpointFact, ObserveBtcChainHeadState, QueryCollectorCheckpointState,
    RecordBtcChainHeadFactState, RecordCollectorCheckpointState,
};
use mfm_program::{InputBindingNodeRef, MfmFactType as _, StateSpec as _};

#[test]
fn cycle_topology_is_deterministic_and_linear() {
    let first =
        btc_chain_head_collector_cycle_program_draft(BtcChainHeadCollectorConfig::default())
            .expect("first draft");
    let second =
        btc_chain_head_collector_cycle_program_draft(BtcChainHeadCollectorConfig::default())
            .expect("second draft");

    assert_eq!(first, second);
    assert_eq!(first.state_nodes().len(), 4);

    let nodes = first.state_nodes();
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>(),
        [
            "query_collector_checkpoint",
            "observe_btc_chain_head",
            "record_btc_chain_head_fact",
            "record_collector_checkpoint",
        ]
    );
    assert_eq!(
        nodes[0].state_kind,
        QueryCollectorCheckpointState::kind().unwrap()
    );
    assert_eq!(
        nodes[1].state_kind,
        ObserveBtcChainHeadState::kind().unwrap()
    );
    assert_eq!(
        nodes[2].state_kind,
        RecordBtcChainHeadFactState::kind().unwrap()
    );
    assert_eq!(
        nodes[3].state_kind,
        RecordCollectorCheckpointState::kind().unwrap()
    );

    assert!(state_input_cells(&nodes[0], nodes).is_empty());
    assert_eq!(
        state_input_cells(&nodes[1], nodes),
        vec![nodes[0].output_cell_id.as_str()]
    );
    assert_eq!(
        state_input_cells(&nodes[2], nodes),
        vec![nodes[1].output_cell_id.as_str()]
    );
    assert_eq!(
        state_input_cells(&nodes[3], nodes),
        vec![
            nodes[2].output_cell_id.as_str(),
            nodes[0].output_cell_id.as_str()
        ]
    );
}

#[test]
fn descriptor_allow_lists_are_attached_to_fact_recording_nodes_and_survive_certification() {
    let draft =
        btc_chain_head_collector_cycle_program_draft(BtcChainHeadCollectorConfig::default())
            .expect("draft");
    let nodes = draft.state_nodes();
    let chain_head_ref =
        mfm_program::fact_descriptor_ref::<BtcChainHeadFact>().expect("chain-head descriptor ref");
    let checkpoint_ref = mfm_program::fact_descriptor_ref::<CollectorCheckpointFact>()
        .expect("checkpoint descriptor ref");
    assert_eq!(
        BtcChainHeadFact::descriptor()
            .expect("chain-head descriptor")
            .fact_kind()
            .as_str(),
        "chain.head"
    );
    assert_eq!(
        CollectorCheckpointFact::descriptor()
            .expect("checkpoint descriptor")
            .fact_kind()
            .as_str(),
        "collector.checkpoint"
    );

    assert!(nodes[0].fact_descriptor_allowlist.is_empty());
    assert!(nodes[1].fact_descriptor_allowlist.is_empty());
    assert_eq!(
        nodes[2].fact_descriptor_allowlist.as_slice(),
        std::slice::from_ref(&chain_head_ref)
    );
    assert_eq!(
        nodes[3].fact_descriptor_allowlist.as_slice(),
        std::slice::from_ref(&checkpoint_ref)
    );
    assert_ne!(
        chain_head_ref.descriptor_hash,
        checkpoint_ref.descriptor_hash
    );

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
    assert_eq!(nodes[3].fact_descriptor_allowlist.len(), 1);
    assert_ne!(
        nodes[2].fact_descriptor_allowlist[0].descriptor_hash,
        nodes[3].fact_descriptor_allowlist[0].descriptor_hash
    );
}

#[test]
fn operation_descriptor_registers() {
    let registry = btc_collectors_operation_registry().expect("operation registry");
    let descriptor = registry
        .operation_descriptor::<BtcChainHeadCollectorCycleOperation>()
        .expect("operation descriptor");

    assert_eq!(
        descriptor.name(),
        "mfm.bitcoin.btc_chain_head_collector_cycle"
    );
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
