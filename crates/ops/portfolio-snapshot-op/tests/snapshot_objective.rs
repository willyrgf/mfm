use mfm_op_portfolio_snapshot::{
    portfolio_snapshot_program_draft, portfolio_snapshot_program_launch_plan,
};
use mfm_portfolio_model::portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use serde_json::json;

const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

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
        "mfm.portfolio.resolve_subjects",
        "mfm.portfolio.select_holdings",
        "mfm.portfolio.resolve_valuations",
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
