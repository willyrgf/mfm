use mfm_op_portfolio_snapshot::{
    portfolio_snapshot_program_draft, portfolio_snapshot_program_launch_plan,
};
use mfm_portfolio_model::portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use mfm_program::TypedProgramDraft;
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
        "mfm.portfolio.collect_evm_network",
        "mfm.portfolio.publish_evm_holdings",
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
    assert!(!operations.iter().any(|name| name.starts_with("mfm.evm.")));

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
    assert_two_state_evm_slice(&bitcoin_only, 0);
    assert_no_state_prefix(&bitcoin_only, "mfm.evm.");

    let evm_native_only =
        deterministic_snapshot_draft(topology_config(false, true, false, false, false));
    assert_eq!(
        operation_count(&evm_native_only, "mfm.bitcoin.btc_network_collection"),
        0
    );
    assert_two_state_evm_slice(&evm_native_only, 1);
    assert_eq!(evm_collection_config(&evm_native_only).sources().len(), 1);
    assert_no_state_prefix(&evm_native_only, "mfm.bitcoin.");
    assert_no_state_prefix(&evm_native_only, "mfm.evm.");

    let evm_token_only =
        deterministic_snapshot_draft(topology_config(false, false, true, false, false));
    assert_eq!(
        operation_count(&evm_token_only, "mfm.bitcoin.btc_network_collection"),
        0
    );
    assert_two_state_evm_slice(&evm_token_only, 1);
    assert_eq!(evm_collection_config(&evm_token_only).sources().len(), 1);
    assert_no_state_prefix(&evm_token_only, "mfm.evm.");

    let mixed = deterministic_snapshot_draft(topology_config(false, true, true, false, false));
    assert_two_state_evm_slice(&mixed, 1);
    assert_eq!(evm_collection_config(&mixed).sources().len(), 2);
    assert_no_state_prefix(&mixed, "mfm.evm.");

    let repeated_token =
        deterministic_snapshot_draft(topology_config(false, false, true, false, true));
    assert_two_state_evm_slice(&repeated_token, 1);
    assert_eq!(evm_collection_config(&repeated_token).sources().len(), 2);

    let unreferenced_token =
        deterministic_snapshot_draft(topology_config(false, true, false, true, false));
    assert_two_state_evm_slice(&unreferenced_token, 1);
    assert_eq!(
        evm_collection_config(&unreferenced_token).sources().len(),
        1
    );
}

fn assert_two_state_evm_slice(draft: &TypedProgramDraft, network_count: usize) {
    assert_eq!(
        state_count(draft, "mfm.portfolio.collect_evm_network"),
        network_count
    );
    assert_eq!(
        state_count(draft, "mfm.portfolio.publish_evm_holdings"),
        network_count
    );
    assert_eq!(
        draft
            .operation_lineage()
            .iter()
            .filter(|operation| operation.operation_name.starts_with("mfm.evm."))
            .count(),
        0
    );
}

fn evm_collection_config(
    draft: &TypedProgramDraft,
) -> mfm_state_portfolio::EvmNetworkCollectionConfig {
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.state_descriptor_name == "mfm.portfolio.collect_evm_network")
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
