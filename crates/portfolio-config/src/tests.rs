use super::*;

use std::path::Path;

use serde_json::json;

fn sample_authored_config() -> PortfolioSnapshotAuthoredConfig {
    serde_json::from_value(sample_request_json()).expect("sample authored config")
}

fn sample_request_json() -> Value {
    json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "unit_price_dec": "1800.00"
                            }
                        ]
                    },
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
}

#[test]
fn authored_json_and_toml_canonicalize_to_same_config_and_hash() {
    let json_authored = parse_portfolio_snapshot_authored_config(
        &sample_request_json().to_string(),
        AuthoredConfigFormat::Json,
    )
    .expect("json authored config");
    let toml_authored = parse_portfolio_snapshot_authored_config_with_hint(
        &toml::to_string(&sample_authored_config()).expect("toml"),
        Some(Path::new("config.toml")),
    )
    .expect("toml authored config");

    let json_canonical =
        canonicalize_portfolio_snapshot_authored_config(json_authored).expect("json canonical");
    let toml_canonical =
        canonicalize_portfolio_snapshot_authored_config(toml_authored).expect("toml canonical");

    assert_eq!(json_canonical, toml_canonical);
    assert_eq!(
        json_canonical.artifact_id().expect("json canonical hash"),
        toml_canonical.artifact_id().expect("toml canonical hash")
    );
}

#[test]
fn decode_canonical_config_accepts_canonical_shape() {
    let canonical = canonicalize_portfolio_snapshot_authored_config(sample_authored_config())
        .expect("canonical");
    let decoded = decode_portfolio_snapshot_canonical_config(
        &serde_json::to_value(&canonical).expect("canonical json"),
    )
    .expect("decode canonical");

    assert_eq!(decoded, canonical);
}
