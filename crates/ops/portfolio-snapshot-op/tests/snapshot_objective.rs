use std::collections::BTreeSet;

use mfm_ids::CellId;
use mfm_op_portfolio_snapshot::{
    portfolio_snapshot_program_draft, portfolio_snapshot_program_launch_plan,
};
use mfm_portfolio_model::portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use mfm_program::{
    BridgeKind, InputBindingNode, InputBindingNodeRef, OperationLineageFrameSpec, StateNodeSpec,
    TypedProgramDraft,
};
use serde_json::json;

const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const EVM_ACCOUNT_B: &str = "0x000000000000000000000000000000000000beef";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";
const UNUSED_TOKEN: &str = "0x0000000000000000000000000000000000000002";

#[test]
fn complete_snapshot_helper_builds_and_certifies_one_root_for_mixed_demand() {
    let config = ValidatedPortfolioConfig::new(mixed_portfolio_config())
        .expect("normalize mixed portfolio")
        .into_config();
    let draft = portfolio_snapshot_program_draft(config.clone()).expect("snapshot draft");

    assert_eq!(draft.root_key().as_str(), "portfolio_snapshot");
    assert_eq!(
        draft.public_output_spec().key().as_str(),
        "portfolio_snapshot"
    );
    assert_eq!(
        draft
            .public_output_spec()
            .outputs()
            .iter()
            .map(|output| output.public_field_path().as_str())
            .collect::<Vec<_>>(),
        ["snapshot", "report"]
    );

    let descriptors = draft
        .state_nodes()
        .iter()
        .map(|node| node.state_descriptor_name.as_str())
        .collect::<Vec<_>>();
    for required in [
        "mfm.portfolio.select_holdings",
        "mfm.bitcoin.collect_balances",
        "mfm.evm.collect_balances",
        "mfm.portfolio.assemble_snapshot",
        "mfm.portfolio.project_report",
    ] {
        assert!(
            descriptors.contains(&required),
            "complete snapshot graph omitted {required}"
        );
    }
    let operations = draft
        .operation_lineage()
        .iter()
        .map(|operation| operation.operation_name.as_str())
        .collect::<Vec<_>>();
    assert!(operations.contains(&"mfm.portfolio.snapshot"));
    assert!(operations.contains(&"mfm.portfolio.report"));
    assert!(operations.contains(&"mfm.bitcoin.balance_collection"));
    assert!(operations.contains(&"mfm.evm.balance_collection"));

    mfm_certify::certify_program_draft(&draft).expect("complete snapshot draft certifies");
    let launch = portfolio_snapshot_program_launch_plan(config).expect("snapshot launch plan");
    assert_eq!(launch.draft, draft);
    assert!(!launch.config_material.is_empty());
}

#[test]
fn snapshot_operation_contains_only_child_operation_composition() {
    let draft = deterministic_snapshot_draft(mixed_portfolio_config());
    let snapshot = operation_frame(&draft, "mfm.portfolio.snapshot");
    let report = operation_frame(&draft, "mfm.portfolio.report");

    assert_eq!(
        report.parent_operation_lineage.active_instances,
        vec![snapshot.operation_instance_id.clone()]
    );
    assert!(draft.state_nodes().iter().all(|node| {
        node.planning_lineage.active_instances != vec![snapshot.operation_instance_id.clone()]
    }));

    let mut direct_children = draft
        .operation_lineage()
        .iter()
        .filter(|operation| {
            operation.parent_operation_lineage.active_instances
                == vec![snapshot.operation_instance_id.clone()]
        })
        .map(|operation| operation.operation_name.as_str())
        .collect::<Vec<_>>();
    direct_children.sort_unstable();
    assert_eq!(
        direct_children,
        [
            "mfm.bitcoin.balance_collection",
            "mfm.evm.balance_collection",
            "mfm.portfolio.report",
        ]
    );
}

#[test]
fn every_report_state_depends_on_all_collector_receipts() {
    let draft = deterministic_snapshot_draft(mixed_portfolio_config());
    let report = operation_frame(&draft, "mfm.portfolio.report");
    let receipt_cells = binding_cell_ids(&report.input.root);
    assert_eq!(
        receipt_cells.len(),
        2,
        "mixed demand has two family receipts"
    );

    let mut report_nodes = draft
        .state_nodes()
        .iter()
        .filter(|node| {
            node.planning_lineage.active_instances.last() == Some(&report.operation_instance_id)
        })
        .collect::<Vec<_>>();
    report_nodes
        .sort_by(|left, right| left.state_descriptor_name.cmp(&right.state_descriptor_name));
    assert_eq!(
        report_nodes
            .iter()
            .map(|node| node.state_descriptor_name.as_str())
            .collect::<Vec<_>>(),
        [
            "mfm.portfolio.assemble_snapshot",
            "mfm.portfolio.project_report",
            "mfm.portfolio.select_holdings",
        ]
    );

    for node in report_nodes {
        let dependencies = transitive_state_input_cells(&draft, node);
        assert!(
            receipt_cells.is_subset(&dependencies),
            "{} did not depend on every collector receipt",
            node.state_descriptor_name
        );
    }
}

#[test]
fn report_selection_receives_only_stored_family_receipts() {
    let draft = deterministic_snapshot_draft(mixed_portfolio_config());
    let report = operation_frame(&draft, "mfm.portfolio.report");
    let selection = draft
        .state_nodes()
        .iter()
        .find(|node| node.state_descriptor_name == "mfm.portfolio.select_holdings")
        .expect("selection state");

    assert_eq!(
        report.input.input_schema_id,
        selection.input.input_schema_id
    );
    assert_eq!(
        report.input.input_descriptor_id,
        selection.input.input_descriptor_id
    );
    assert_eq!(report.input.root, selection.input.root);
    assert_eq!(report.input.digest, selection.input.digest);

    let receipt_cells = binding_cell_ids(&selection.input.root);
    let mut receipt_producers = Vec::new();
    for receipt_cell in receipt_cells {
        let bridge = draft
            .bridge_nodes()
            .iter()
            .find(|bridge| bridge.target_cell_id == receipt_cell)
            .expect("report receipt must be exported from a collector child scope");
        assert_eq!(bridge.bridge_kind, BridgeKind::ExportToParent);
        let producer = draft
            .state_nodes()
            .iter()
            .find(|node| node.output_cell_id == bridge.source_cell_id)
            .expect("collector receipt source must be a state output");
        receipt_producers.push(producer.state_descriptor_name.as_str());
    }
    receipt_producers.sort_unstable();
    assert_eq!(
        receipt_producers,
        ["mfm.bitcoin.collect_balances", "mfm.evm.collect_balances",]
    );
}

/// The aggregate expands only the requested family work. Draft equality proves the complete
/// graph, including child scopes, bridges, edges, and certified configs, remains deterministic.
#[test]
fn aggregate_snapshot_topology_matches_demand_without_inert_collection_nodes() {
    let bitcoin_only =
        deterministic_snapshot_draft(topology_config(true, false, false, false, false));
    assert_eq!(
        operation_count(&bitcoin_only, "mfm.bitcoin.balance_collection"),
        1
    );
    assert_one_state_evm_slice(&bitcoin_only, 0);
    assert_no_state_prefix(&bitcoin_only, "mfm.evm.");

    let evm_native_only =
        deterministic_snapshot_draft(topology_config(false, true, false, false, false));
    assert_eq!(
        operation_count(&evm_native_only, "mfm.bitcoin.balance_collection"),
        0
    );
    assert_one_state_evm_slice(&evm_native_only, 1);
    assert_eq!(evm_collection_config(&evm_native_only).sources().len(), 1);
    assert_no_state_prefix(&evm_native_only, "mfm.bitcoin.");

    let evm_token_only =
        deterministic_snapshot_draft(topology_config(false, false, true, false, false));
    assert_eq!(
        operation_count(&evm_token_only, "mfm.bitcoin.balance_collection"),
        0
    );
    assert_one_state_evm_slice(&evm_token_only, 1);
    assert_eq!(evm_collection_config(&evm_token_only).sources().len(), 1);

    let mixed = deterministic_snapshot_draft(topology_config(false, true, true, false, false));
    assert_one_state_evm_slice(&mixed, 1);
    assert_eq!(evm_collection_config(&mixed).sources().len(), 2);

    let repeated_token =
        deterministic_snapshot_draft(topology_config(false, false, true, false, true));
    assert_one_state_evm_slice(&repeated_token, 1);
    assert_eq!(evm_collection_config(&repeated_token).sources().len(), 2);

    let unreferenced_token =
        deterministic_snapshot_draft(topology_config(false, true, false, true, false));
    assert_one_state_evm_slice(&unreferenced_token, 1);
    assert_eq!(
        evm_collection_config(&unreferenced_token).sources().len(),
        1
    );
}

fn assert_one_state_evm_slice(draft: &TypedProgramDraft, network_count: usize) {
    assert_eq!(
        state_count(draft, "mfm.evm.collect_balances"),
        network_count
    );
    assert_eq!(
        draft
            .operation_lineage()
            .iter()
            .filter(|operation| operation.operation_name.starts_with("mfm.evm."))
            .count(),
        network_count
    );
}

fn evm_collection_config(draft: &TypedProgramDraft) -> mfm_evm::EvmBalanceCollectionConfig {
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.state_descriptor_name == "mfm.evm.collect_balances")
        .expect("EVM collection node");
    serde_json::from_slice(node.config.canonical_json.as_bytes()).expect("EVM collection config")
}

fn deterministic_snapshot_draft(config: PortfolioConfig) -> TypedProgramDraft {
    let normalized = ValidatedPortfolioConfig::new(config)
        .expect("normalize topology fixture")
        .into_config();
    let first = portfolio_snapshot_program_draft(normalized.clone()).expect("first snapshot draft");
    let second = portfolio_snapshot_program_draft(normalized).expect("second snapshot draft");
    assert_eq!(
        first, second,
        "repeated aggregate expansion must preserve keys, edges, bindings, and certified configs"
    );
    mfm_certify::certify_program_draft(&first).expect("topology draft certifies");
    first
}

fn operation_count(draft: &TypedProgramDraft, name: &str) -> usize {
    draft
        .operation_lineage()
        .iter()
        .filter(|operation| operation.operation_name == name)
        .count()
}

fn operation_frame<'a>(draft: &'a TypedProgramDraft, name: &str) -> &'a OperationLineageFrameSpec {
    let mut matching = draft
        .operation_lineage()
        .iter()
        .filter(|operation| operation.operation_name == name);
    let frame = matching.next().expect("required operation frame");
    assert!(matching.next().is_none(), "operation frame must be unique");
    frame
}

fn binding_cell_ids(root: &InputBindingNode) -> BTreeSet<CellId> {
    fn collect(node: &InputBindingNode, cells: &mut BTreeSet<CellId>) {
        match node.as_ref() {
            InputBindingNodeRef::Unit => {}
            InputBindingNodeRef::Cell(cell) => {
                cells.insert(cell.cell_id().clone());
            }
            InputBindingNodeRef::Tuple(elements)
            | InputBindingNodeRef::Vec { elements, .. }
            | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
                for element in elements {
                    collect(element, cells);
                }
            }
            InputBindingNodeRef::Struct(fields) => {
                for field in fields {
                    collect(&field.node, cells);
                }
            }
        }
    }

    let mut cells = BTreeSet::new();
    collect(root, &mut cells);
    cells
}

fn transitive_state_input_cells(
    draft: &TypedProgramDraft,
    node: &StateNodeSpec,
) -> BTreeSet<CellId> {
    fn visit(draft: &TypedProgramDraft, cell: CellId, visited: &mut BTreeSet<CellId>) {
        if !visited.insert(cell.clone()) {
            return;
        }
        if let Some(producer) = draft
            .state_nodes()
            .iter()
            .find(|candidate| candidate.output_cell_id == cell)
        {
            for input in binding_cell_ids(&producer.input.root) {
                visit(draft, input, visited);
            }
        }
    }

    let mut visited = BTreeSet::new();
    for input in binding_cell_ids(&node.input.root) {
        visit(draft, input, &mut visited);
    }
    visited
}

fn state_count(draft: &TypedProgramDraft, name: &str) -> usize {
    draft
        .state_nodes()
        .iter()
        .filter(|node| node.state_descriptor_name == name)
        .count()
}

fn assert_no_state_prefix(draft: &TypedProgramDraft, prefix: &str) {
    assert!(
        draft
            .state_nodes()
            .iter()
            .all(|node| !node.state_descriptor_name.starts_with(prefix)),
        "aggregate graph unexpectedly contained a {prefix} state"
    );
}

fn mixed_portfolio_config() -> PortfolioConfig {
    serde_json::from_value(json!({
        "portfolio_id": "snapshot-objective-test",
        "quote_codes": ["USD"],
        "networks": [
            {
                "network_id": "bitcoin-mainnet",
                "family": "bitcoin",
                "bitcoin_network": "main",
                "source_identity": "public-bitcoin-core",
                "metadata": {}
            },
            {
                "network_id": "ethereum-mainnet",
                "family": "evm",
                "chain_id": 1,
                "native_decimals": 18,
                "metadata": {}
            }
        ],
        "wallets": [
            {
                "wallet_id": "wallet_btc",
                "network_id": "bitcoin-mainnet",
                "symbol_ids": ["btc.native.bitcoin-mainnet"],
                "subject": {"kind": "bitcoin_address", "address": BTC_ADDRESS},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            },
            {
                "wallet_id": "wallet_eth",
                "network_id": "ethereum-mainnet",
                "symbol_ids": ["eth.native.ethereum-mainnet", "usdc.ethereum-mainnet"],
                "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }
        ],
        "symbol_configs": [
            symbol("btc.native.bitcoin-mainnet", "bitcoin-mainnet", json!({"kind": "native"})),
            symbol("eth.native.ethereum-mainnet", "ethereum-mainnet", json!({"kind": "native"})),
            symbol("usdc.ethereum-mainnet", "ethereum-mainnet", json!({
                "kind": "erc20",
                "contract_address": TOKEN
            }))
        ],
        "metadata": {}
    }))
    .expect("valid portfolio fixture")
}

fn topology_config(
    include_btc: bool,
    include_evm_native: bool,
    include_evm_token: bool,
    include_unreferenced_token: bool,
    duplicate_token_account: bool,
) -> PortfolioConfig {
    let mut networks = Vec::new();
    let mut wallets = Vec::new();
    let mut symbols = Vec::new();

    if include_evm_native || include_evm_token || include_unreferenced_token {
        networks.push(json!({
            "network_id": "ethereum-mainnet",
            "family": "evm",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {}
        }));
        let mut symbol_ids = Vec::new();
        if include_evm_native {
            symbol_ids.push("eth.native.ethereum-mainnet");
            symbols.push(symbol(
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "native"}),
            ));
        }
        if include_evm_token {
            symbol_ids.push("usdc.ethereum-mainnet");
            symbols.push(symbol(
                "usdc.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "erc20", "contract_address": TOKEN}),
            ));
        }
        if include_unreferenced_token {
            symbols.push(symbol(
                "unused.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "erc20", "contract_address": UNUSED_TOKEN}),
            ));
        }
        wallets.push(json!({
            "wallet_id": "wallet_eth_a",
            "network_id": "ethereum-mainnet",
            "symbol_ids": symbol_ids,
            "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
            "implementation": {"kind": "address_only"},
            "metadata": {}
        }));
        if duplicate_token_account {
            wallets.push(json!({
                "wallet_id": "wallet_eth_b",
                "network_id": "ethereum-mainnet",
                "symbol_ids": ["usdc.ethereum-mainnet"],
                "subject": {"kind": "evm_address", "address": EVM_ACCOUNT_B},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
        }
    }

    if include_btc {
        networks.push(json!({
            "network_id": "bitcoin-mainnet",
            "family": "bitcoin",
            "bitcoin_network": "main",
            "source_identity": "public-bitcoin-core",
            "metadata": {}
        }));
        wallets.push(json!({
            "wallet_id": "wallet_btc",
            "network_id": "bitcoin-mainnet",
            "symbol_ids": ["btc.native.bitcoin-mainnet"],
            "subject": {"kind": "bitcoin_address", "address": BTC_ADDRESS},
            "implementation": {"kind": "address_only"},
            "metadata": {}
        }));
        symbols.push(symbol(
            "btc.native.bitcoin-mainnet",
            "bitcoin-mainnet",
            json!({"kind": "native"}),
        ));
    }

    serde_json::from_value(json!({
        "portfolio_id": "snapshot-topology-test",
        "quote_codes": ["USD"],
        "networks": networks,
        "wallets": wallets,
        "symbol_configs": symbols,
        "metadata": {}
    }))
    .expect("valid topology fixture")
}

fn symbol(symbol_id: &str, network_id: &str, source: serde_json::Value) -> serde_json::Value {
    json!({
        "symbol_id": symbol_id,
        "display_symbol": symbol_id,
        "network_id": network_id,
        "source": source,
        "valuation": {
            "quotes": [{
                "quote": "USD",
                "priced_symbol_id": symbol_id,
                "unit_price_dec": "1.00"
            }]
        },
        "metadata": {}
    })
}
