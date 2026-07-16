use mfm_op_portfolio_snapshot::{
    portfolio_snapshot_program_draft, portfolio_snapshot_program_launch_plan,
};
use mfm_portfolio_model::portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use mfm_program::{InputBindingNodeRef, TypedProgramDraft};
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
        "mfm.portfolio.assemble_collection_receipt",
        "mfm.portfolio.select_holdings",
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
    assert!(operations.contains(&"mfm.bitcoin.btc_network_collection"));
    assert!(operations.contains(&"mfm.evm.evm_network_collection"));

    mfm_certify::certify_program_draft(&draft).expect("complete snapshot draft certifies");
    let launch = portfolio_snapshot_program_launch_plan(config).expect("snapshot launch plan");
    assert_eq!(launch.draft, draft);
    assert!(!launch.config_material.is_empty());
}

/// The aggregate expands only the requested family work. Draft equality proves the complete
/// graph, including child scopes, bridges, edges, and certified configs, remains deterministic.
#[test]
fn aggregate_snapshot_topology_matches_demand_without_inert_collection_nodes() {
    let bitcoin_only =
        deterministic_snapshot_draft(topology_config(true, false, false, false, false));
    assert_eq!(
        operation_count(&bitcoin_only, "mfm.bitcoin.btc_network_collection"),
        1
    );
    assert_eq!(
        operation_count(&bitcoin_only, "mfm.evm.evm_network_collection"),
        0
    );
    assert_no_state_prefix(&bitcoin_only, "mfm.evm.");

    let evm_native_only =
        deterministic_snapshot_draft(topology_config(false, true, false, false, false));
    assert_eq!(
        operation_count(&evm_native_only, "mfm.bitcoin.btc_network_collection"),
        0
    );
    assert_eq!(
        operation_count(&evm_native_only, "mfm.evm.evm_network_collection"),
        1
    );
    assert_eq!(
        operation_count(&evm_native_only, "mfm.evm.evm_native_balances_at_anchor"),
        1
    );
    assert_eq!(
        operation_count(&evm_native_only, "mfm.evm.evm_erc20_balances_at_anchor"),
        0
    );
    assert_eq!(
        state_count(&evm_native_only, "mfm.evm.joint_tip.resolve"),
        1
    );
    assert_eq!(
        state_count(&evm_native_only, "mfm.evm.native_balance.observe"),
        1
    );
    assert_no_state_fragment(&evm_native_only, "erc20");
    assert_no_state_prefix(&evm_native_only, "mfm.bitcoin.");

    let evm_token_only =
        deterministic_snapshot_draft(topology_config(false, false, true, false, false));
    assert_eq!(
        operation_count(&evm_token_only, "mfm.bitcoin.btc_network_collection"),
        0
    );
    assert_eq!(
        operation_count(&evm_token_only, "mfm.evm.evm_network_collection"),
        1
    );
    assert_eq!(
        operation_count(&evm_token_only, "mfm.evm.evm_native_balances_at_anchor"),
        0
    );
    assert_eq!(
        operation_count(&evm_token_only, "mfm.evm.evm_erc20_balances_at_anchor"),
        1
    );
    assert_eq!(state_count(&evm_token_only, "mfm.evm.joint_tip.resolve"), 1);
    assert_eq!(
        state_count(&evm_token_only, "mfm.evm.erc20_token_metadata.observe"),
        1
    );
    assert_eq!(
        state_count(&evm_token_only, "mfm.evm.erc20_balance.observe"),
        1
    );
    assert_no_state_fragment(&evm_token_only, "native_balance");

    let mixed = deterministic_snapshot_draft(topology_config(false, true, true, false, false));
    assert_eq!(operation_count(&mixed, "mfm.evm.evm_network_collection"), 1);
    assert_eq!(
        operation_count(&mixed, "mfm.evm.evm_native_balances_at_anchor"),
        1
    );
    assert_eq!(
        operation_count(&mixed, "mfm.evm.evm_erc20_balances_at_anchor"),
        1
    );
    assert_eq!(
        state_count(&mixed, "mfm.evm.joint_tip.resolve"),
        1,
        "native and ERC-20 children must share the network coordinator's sole joint tip"
    );
    assert_eq!(state_count(&mixed, "mfm.evm.native_balance.observe"), 1);
    assert_eq!(
        state_count(&mixed, "mfm.evm.erc20_token_metadata.observe"),
        1
    );
    assert_eq!(state_count(&mixed, "mfm.evm.erc20_balance.observe"), 1);
    let tip_cell = mixed
        .state_nodes()
        .iter()
        .find(|node| node.state_descriptor_name == "mfm.evm.joint_tip.resolve")
        .expect("mixed graph joint-tip node")
        .output_cell_id
        .as_str();
    for descriptor in [
        "mfm.evm.native_balance.observe",
        "mfm.evm.erc20_token_metadata.observe",
        "mfm.evm.erc20_balance.observe",
    ] {
        for node in mixed
            .state_nodes()
            .iter()
            .filter(|node| node.state_descriptor_name == descriptor)
        {
            assert!(
                state_input_traces_to_cell(&mixed, node, tip_cell),
                "{descriptor} must consume the sole EVM joint-tip lineage through certified bridges"
            );
        }
    }

    let repeated_token =
        deterministic_snapshot_draft(topology_config(false, false, true, false, true));
    assert_eq!(
        state_count(&repeated_token, "mfm.evm.erc20_token_metadata.observe"),
        1,
        "one token contract receives one metadata read per network"
    );
    assert_eq!(
        state_count(&repeated_token, "mfm.evm.erc20_balance.observe"),
        2,
        "each demanded token account receives one balance read"
    );

    let unreferenced_token =
        deterministic_snapshot_draft(topology_config(false, true, false, true, false));
    assert_eq!(
        operation_count(&unreferenced_token, "mfm.evm.evm_erc20_balances_at_anchor"),
        0
    );
    assert_no_state_fragment(&unreferenced_token, "erc20");
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

fn assert_no_state_fragment(draft: &TypedProgramDraft, fragment: &str) {
    assert!(
        draft
            .state_nodes()
            .iter()
            .all(|node| !node.state_descriptor_name.contains(fragment)),
        "aggregate graph unexpectedly contained a state matching {fragment}"
    );
}

fn state_input_traces_to_cell(
    draft: &TypedProgramDraft,
    node: &mfm_program::StateNodeSpec,
    source_cell: &str,
) -> bool {
    let mut cells = Vec::new();
    collect_input_cells(node.input.root.as_ref(), &mut cells);
    cells
        .into_iter()
        .any(|cell| bridge_lineage_traces_to_cell(draft, cell, source_cell))
}

fn bridge_lineage_traces_to_cell(
    draft: &TypedProgramDraft,
    input_cell: &str,
    source_cell: &str,
) -> bool {
    let mut current = input_cell;
    for _ in 0..=draft.bridge_nodes().len() {
        if current == source_cell {
            return true;
        }
        let Some(bridge) = draft
            .bridge_nodes()
            .iter()
            .find(|bridge| bridge.target_cell_id.as_str() == current)
        else {
            return false;
        };
        current = bridge.source_cell_id.as_str();
    }
    false
}

fn collect_input_cells<'a>(node: InputBindingNodeRef<'a>, cells: &mut Vec<&'a str>) {
    match node {
        InputBindingNodeRef::Unit => {}
        InputBindingNodeRef::Cell(cell) => cells.push(cell.cell_id().as_str()),
        InputBindingNodeRef::Tuple(elements) => {
            for element in elements {
                collect_input_cells(element.as_ref(), cells);
            }
        }
        InputBindingNodeRef::Struct(fields) => {
            for field in fields {
                collect_input_cells(field.node.as_ref(), cells);
            }
        }
        InputBindingNodeRef::Vec { elements, .. }
        | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element.as_ref(), cells);
            }
        }
    }
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
