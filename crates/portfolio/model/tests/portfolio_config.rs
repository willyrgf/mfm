use std::collections::BTreeMap;

use mfm_portfolio_model::portfolio::{
    decode_portfolio_config, NetworkConfig, NetworkFamilyConfig, PortfolioConfigError,
    EVM_NETWORK_HOLDING_SOURCE_LIMIT, PORTFOLIO_HOLDING_RELATION_LIMIT, PORTFOLIO_NETWORK_LIMIT,
    PORTFOLIO_SYMBOL_LIMIT, PORTFOLIO_WALLET_LIMIT,
};
use mfm_portfolio_model::symbol::{QuoteCode, SymbolConfigError};
use serde_json::{json, Value};

const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

#[test]
fn direct_model_accepts_btc_native_evm_native_and_erc20() {
    let config = decode_portfolio_config(&canonical_config()).expect("valid direct model");

    assert_eq!(config.quote_codes, vec![QuoteCode::Usd]);
    assert_eq!(config.networks.len(), 2);
    assert_eq!(config.wallets.len(), 2);
    assert_eq!(config.symbol_configs.len(), 3);
    assert_eq!(config.wallets[1].symbol_ids.len(), 2);
    assert_eq!(
        config
            .networks
            .iter()
            .find(|network| network.network_id().as_str() == "ethereum-mainnet")
            .expect("EVM network")
            .native_decimals(),
        Some(18)
    );
}

#[test]
fn evm_token_only_wallet_and_unreferenced_symbol_are_valid() {
    let mut token_only = canonical_config();
    token_only["wallets"].as_array_mut().expect("wallets")[1]["symbol_ids"] =
        json!(["usdc.wallet.ethereum-mainnet"]);
    let config = decode_portfolio_config(&token_only).expect("token-only wallet is valid");
    assert_eq!(config.wallets[1].symbol_ids.len(), 1);

    let mut with_unreferenced = canonical_config();
    with_unreferenced["symbol_configs"]
        .as_array_mut()
        .expect("symbols")
        .push(symbol(
            "eth.unreferenced.ethereum-mainnet",
            "ethereum-mainnet",
            json!({"kind": "native"}),
        ));
    let config = decode_portfolio_config(&with_unreferenced)
        .expect("unreferenced symbols do not create demand");
    assert!(config
        .symbol_configs
        .iter()
        .any(|symbol| symbol.symbol_id.as_str() == "eth.unreferenced.ethereum-mainnet"));
}

#[test]
fn no_explicit_wallet_to_symbol_edges_is_rejected() {
    let mut config = canonical_config();
    for wallet in config["wallets"].as_array_mut().expect("wallets") {
        wallet["symbol_ids"] = json!([]);
    }

    assert_error(&config, PortfolioConfigError::EmptyHoldingDemand);
}

#[test]
fn source_family_matrix_and_wallet_network_joins_are_enforced() {
    let mut btc_erc20 = canonical_config();
    btc_erc20["symbol_configs"].as_array_mut().expect("symbols")[0]["source"] =
        json!({"kind": "erc20", "contract_address": TOKEN});
    assert!(matches!(
        decode_portfolio_config(&btc_erc20),
        Err(PortfolioConfigError::UnsupportedHoldingSourceNetworkFamily { .. })
    ));

    let mut subject_mismatch = canonical_config();
    subject_mismatch["wallets"].as_array_mut().expect("wallets")[1]["subject"] = json!({
        "kind": "bitcoin_address",
        "address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"
    });
    assert!(matches!(
        decode_portfolio_config(&subject_mismatch),
        Err(PortfolioConfigError::WalletSubjectNetworkFamilyMismatch { .. })
    ));

    let mut symbol_mismatch = canonical_config();
    symbol_mismatch["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[1]["network_id"] = json!("bitcoin-mainnet");
    assert!(matches!(
        decode_portfolio_config(&symbol_mismatch),
        Err(PortfolioConfigError::WalletSymbolNetworkMismatch { .. })
    ));
}

#[test]
fn bitcoin_wallet_addresses_follow_checked_network_encoding() {
    for (network, address) in [
        ("main", "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"),
        (
            "test",
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        (
            "testnet4",
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        (
            "signet",
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        ("regtest", "bcrt1q2nfxmhd4n3c8834pj72xagvyr9gl57n5r94fsl"),
    ] {
        let mut config = canonical_config();
        config["networks"][0]["bitcoin_network"] = json!(network);
        config["wallets"][0]["subject"]["address"] = json!(address);
        decode_portfolio_config(&config)
            .unwrap_or_else(|error| panic!("{network} address should be compatible: {error}"));
    }

    let mut wrong_family = canonical_config();
    wrong_family["networks"][0]["bitcoin_network"] = json!("testnet4");
    assert!(matches!(
        decode_portfolio_config(&wrong_family),
        Err(PortfolioConfigError::BitcoinWalletAddressNetworkMismatch { .. })
    ));
}

#[test]
fn token_address_and_source_aliases_fail_closed() {
    let mut zero_token = canonical_config();
    zero_token["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[2]["source"]["contract_address"] =
        json!("0x0000000000000000000000000000000000000000");
    assert!(matches!(
        decode_portfolio_config(&zero_token),
        Err(PortfolioConfigError::InvalidSymbolConfig {
            source,
            ..
        }) if *source == SymbolConfigError::ZeroErc20ContractAddress
    ));

    let mut non_normalized = canonical_config();
    non_normalized["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[2]["source"]["contract_address"] =
        json!("0x00000000000000000000000000000000000000AA");
    assert!(matches!(
        decode_portfolio_config(&non_normalized),
        Err(PortfolioConfigError::Decode(_))
    ));

    let mut native_null_contract = canonical_config();
    native_null_contract["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[1]["source"]["contract_address"] = Value::Null;
    assert!(matches!(
        decode_portfolio_config(&native_null_contract),
        Err(PortfolioConfigError::Decode(_))
    ));

    let mut alias = canonical_config();
    alias["symbol_configs"]
        .as_array_mut()
        .expect("symbols")
        .push(symbol(
            "usdc.alias.ethereum-mainnet",
            "ethereum-mainnet",
            json!({"kind": "erc20", "contract_address": TOKEN}),
        ));
    alias["wallets"].as_array_mut().expect("wallets")[1]["symbol_ids"] = json!([
        "eth.native.ethereum-mainnet",
        "usdc.wallet.ethereum-mainnet",
        "usdc.alias.ethereum-mainnet"
    ]);
    assert!(matches!(
        decode_portfolio_config(&alias),
        Err(PortfolioConfigError::AliasedHoldingSource { .. })
    ));
}

#[test]
fn duplicate_semantic_wallet_subject_is_rejected() {
    let mut config = canonical_config();
    let mut duplicate = config["wallets"][1].clone();
    duplicate["wallet_id"] = json!("wallet_evm_alias");
    config["wallets"]
        .as_array_mut()
        .expect("wallets")
        .push(duplicate);

    assert!(matches!(
        decode_portfolio_config(&config),
        Err(PortfolioConfigError::DuplicateWalletSubject { .. })
    ));
}

#[test]
fn valuation_routes_are_complete_and_float_free() {
    let mut incomplete = canonical_config();
    incomplete["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[1]["valuation"]["quotes"] = json!([]);
    assert!(matches!(
        decode_portfolio_config(&incomplete),
        Err(PortfolioConfigError::MissingValuationQuote { .. })
    ));

    let mut float_valued = canonical_config();
    float_valued["symbol_configs"]
        .as_array_mut()
        .expect("symbols")[1]["valuation"]["quotes"][0]["unit_price_dec"] = json!(1800.5);
    assert!(matches!(
        decode_portfolio_config(&float_valued),
        Err(PortfolioConfigError::Decode(_))
    ));
}

#[test]
fn equivalent_input_orderings_normalize_to_the_same_config_and_demand_order() {
    let canonical = decode_portfolio_config(&canonical_config()).expect("canonical config");
    let mut reordered = canonical_config();
    reordered["networks"]
        .as_array_mut()
        .expect("networks")
        .reverse();
    reordered["wallets"]
        .as_array_mut()
        .expect("wallets")
        .reverse();
    reordered["symbol_configs"]
        .as_array_mut()
        .expect("symbols")
        .reverse();
    reordered["wallets"].as_array_mut().expect("wallets")[0]["symbol_ids"] = json!([
        "usdc.wallet.ethereum-mainnet",
        "eth.native.ethereum-mainnet"
    ]);

    let normalized = decode_portfolio_config(&reordered).expect("reordered config");
    assert_eq!(canonical, normalized);
    assert_eq!(
        normalized.wallets[1]
            .symbol_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![
            "eth.native.ethereum-mainnet".to_owned(),
            "usdc.wallet.ethereum-mainnet".to_owned()
        ]
    );
}

#[test]
fn evm_native_scale_is_explicit_and_bitcoin_cannot_carry_one() {
    assert!(matches!(
        NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            None,
            None,
            None,
            BTreeMap::new(),
        ),
        Err(PortfolioConfigError::MissingEvmNativeDecimals { .. })
    ));
    assert!(matches!(
        NetworkConfig::new(
            "bitcoin-mainnet".to_owned(),
            NetworkFamilyConfig::Bitcoin,
            None,
            Some(8),
            Some("main".to_owned()),
            Some("public-bitcoin-core".to_owned()),
            BTreeMap::new(),
        ),
        Err(PortfolioConfigError::UnexpectedBitcoinNativeDecimals { .. })
    ));
}

#[test]
fn portfolio_cardinality_bounds_fail_before_graph_expansion() {
    for (field, limit, collection) in [
        ("networks", PORTFOLIO_NETWORK_LIMIT, "network"),
        ("wallets", PORTFOLIO_WALLET_LIMIT, "wallet"),
        ("symbol_configs", PORTFOLIO_SYMBOL_LIMIT, "symbol"),
    ] {
        let mut config = canonical_config();
        let values = config[field].as_array_mut().expect("bounded collection");
        let fixture = values[0].clone();
        values.resize(limit + 1, fixture);
        assert!(matches!(
            decode_portfolio_config(&config),
            Err(PortfolioConfigError::CollectionLimitExceeded {
                collection: actual_collection,
                limit: actual_limit,
                actual,
            }) if actual_collection == collection && actual_limit == limit && actual == limit + 1
        ));
    }

    let mut relations = canonical_config();
    relations["wallets"][1]["symbol_ids"] = json!(std::iter::repeat_n(
        "eth.native.ethereum-mainnet",
        PORTFOLIO_HOLDING_RELATION_LIMIT + 1
    )
    .collect::<Vec<_>>());
    assert!(matches!(
        decode_portfolio_config(&relations),
        Err(PortfolioConfigError::CollectionLimitExceeded {
            collection: "holding relation",
            limit: PORTFOLIO_HOLDING_RELATION_LIMIT,
            ..
        })
    ));
}

#[test]
fn per_evm_network_holding_source_bound_is_admitted_once_in_portfolio_validation() {
    let mut config = canonical_config();
    config["wallets"][1]["symbol_ids"] = json!(std::iter::repeat_n(
        "eth.native.ethereum-mainnet",
        EVM_NETWORK_HOLDING_SOURCE_LIMIT + 1
    )
    .collect::<Vec<_>>());
    assert!(matches!(
        decode_portfolio_config(&config),
        Err(PortfolioConfigError::EvmNetworkHoldingSourceLimitExceeded {
            network_id,
            limit: EVM_NETWORK_HOLDING_SOURCE_LIMIT,
            actual,
        }) if network_id == "ethereum-mainnet"
            && actual == EVM_NETWORK_HOLDING_SOURCE_LIMIT + 1
    ));
}

fn assert_error(value: &Value, expected: PortfolioConfigError) {
    assert_eq!(
        decode_portfolio_config(value).expect_err("config should fail"),
        expected
    );
}

fn canonical_config() -> Value {
    json!({
        "portfolio_id": "portfolio_main",
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
                "subject": {
                    "kind": "bitcoin_address",
                    "address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"
                },
                "network_id": "bitcoin-mainnet",
                "implementation": {"kind": "address_only"},
                "symbol_ids": ["btc.native.bitcoin-mainnet"],
                "metadata": {}
            },
            {
                "wallet_id": "wallet_evm",
                "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
                "network_id": "ethereum-mainnet",
                "implementation": {"kind": "address_only"},
                "symbol_ids": [
                    "eth.native.ethereum-mainnet",
                    "usdc.wallet.ethereum-mainnet"
                ],
                "metadata": {}
            }
        ],
        "symbol_configs": [
            symbol("btc.native.bitcoin-mainnet", "bitcoin-mainnet", json!({"kind": "native"})),
            symbol("eth.native.ethereum-mainnet", "ethereum-mainnet", json!({"kind": "native"})),
            symbol(
                "usdc.wallet.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "erc20", "contract_address": TOKEN})
            )
        ],
        "metadata": {}
    })
}

fn symbol(symbol_id: &str, network_id: &str, source: Value) -> Value {
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
