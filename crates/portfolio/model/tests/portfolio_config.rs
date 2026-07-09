use std::collections::BTreeMap;

use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::*;
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, Observation, ObservationAnchor, ObservationQuantity, ObservationSource,
    ObservationValue, QuoteCode, SymbolKind, SymbolRole,
};
use mfm_portfolio_model::wallet::{WalletImplementationConfig, WalletSubjectKind};
use serde_json::{json, Value};

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn decode_canonical_portfolio_config() {
    let cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");

    assert_eq!(cfg.portfolio_id, "portfolio_main");
    assert_eq!(cfg.quote_codes, vec![QuoteCode::Btc, QuoteCode::Usd]);
    assert_eq!(cfg.networks.len(), 2);
    assert_eq!(cfg.wallets.len(), 2);
    assert_eq!(cfg.symbol_configs.len(), 3);
    assert!(matches!(
        cfg.wallets[0].implementation,
        WalletImplementationConfig::AddressOnly {}
    ));
    assert_eq!(cfg.wallets[0].wallet_id, "wallet_ops_arb");
    assert_eq!(
        cfg.wallets[0].subject.address_str(),
        "0x000000000000000000000000000000000000beef"
    );
    assert!(matches!(
        cfg.symbol_configs[0].balance_reader,
        BalanceReaderConfig::NativeBalance {}
    ));
}

#[test]
fn validated_config_wrappers_roundtrip_and_reject_duplicate_ids() {
    let network = NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("network config");

    let authority = ValidatedNetworkConfigs::new(vec![network.clone()]).expect("network authority");
    assert_eq!(authority.as_slice(), std::slice::from_ref(&network));
    assert_eq!(authority.into_vec(), vec![network.clone()]);

    assert!(matches!(
        ValidatedNetworkConfigs::new(vec![network.clone(), network]),
        Err(PortfolioConfigError::DuplicateNetworkId { network_id })
            if network_id == "ethereum-mainnet"
    ));

    let cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");
    let wallet = cfg.wallets[0].clone();
    let symbol = cfg.symbol_configs[0].clone();

    let wallets = ValidatedWalletConfigs::new(vec![wallet.clone()]).expect("wallets");
    assert_eq!(wallets.as_slice(), std::slice::from_ref(&wallet));
    assert_eq!(wallets.into_vec(), vec![wallet.clone()]);
    assert!(matches!(
        ValidatedWalletConfigs::new(vec![wallet.clone(), wallet]),
        Err(PortfolioConfigError::DuplicateWalletId { wallet_id })
            if wallet_id == "wallet_ops_arb"
    ));

    let expected_symbol_id = symbol.symbol_id.clone();
    let symbols = ValidatedSymbolConfigs::new(vec![symbol.clone()]).expect("symbols");
    assert_eq!(symbols.as_slice(), std::slice::from_ref(&symbol));
    assert_eq!(symbols.into_vec(), vec![symbol.clone()]);
    assert!(matches!(
        ValidatedSymbolConfigs::new(vec![symbol.clone(), symbol]),
        Err(PortfolioConfigError::DuplicateSymbolId { symbol_id })
            if symbol_id == expected_symbol_id.as_str()
    ));
}

#[test]
fn network_config_uses_bitcoin_source_identity_only() {
    let evm_with_source = NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        None,
        Some("ethereum-mainnet".to_owned()),
        BTreeMap::new(),
    )
    .expect_err("EVM source identity must be rejected");
    assert!(matches!(
        evm_with_source,
        PortfolioConfigError::UnexpectedEvmSourceIdentity { network_id }
            if network_id == "ethereum-mainnet"
    ));

    let bitcoin_missing_source = NetworkConfig::new(
        "bitcoin-mainnet".to_owned(),
        NetworkFamilyConfig::Bitcoin,
        None,
        Some("main".to_owned()),
        None,
        BTreeMap::new(),
    )
    .expect_err("Bitcoin source identity is required");
    assert!(matches!(
        bitcoin_missing_source,
        PortfolioConfigError::MissingBitcoinSourceIdentity { network_id }
            if network_id == "bitcoin-mainnet"
    ));

    let bitcoin_invalid_source = NetworkConfig::new(
        "bitcoin-mainnet".to_owned(),
        NetworkFamilyConfig::Bitcoin,
        None,
        Some("main".to_owned()),
        Some("portfolio/main-wallet".to_owned()),
        BTreeMap::new(),
    )
    .expect_err("Bitcoin source identity uses local public id grammar");
    assert!(matches!(
        bitcoin_invalid_source,
        PortfolioConfigError::InvalidBitcoinSourceIdentity { network_id, .. }
            if network_id == "bitcoin-mainnet"
    ));
}

#[test]
fn validated_portfolio_config_indexes_normalized_authority() {
    let portfolio = decode_portfolio_config(&json!({
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
                "network_id": "ethereum-mainnet",
                "implementation": {"kind": "address_only"},
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
                "balance_reader": {"kind": "native_balance"},
                "valuation": {
                    "quotes": [
                        {
                            "quote": "USD",
                            "priced_symbol_id": "eth.native.ethereum-mainnet",
                            "unit_price_dec": "1800.00"
                        }
                    ]
                },
                "decimals": 18,
                "underlying_symbol_id": null,
                "metadata": {}
            }
        ],
        "metadata": {}
    }))
    .expect("portfolio config");
    let validated = ValidatedPortfolioConfig::new(portfolio).expect("validated portfolio");

    assert_eq!(validated.quote(QuoteCode::Usd), Some(QuoteCode::Usd));
    assert_eq!(
        validated
            .network("ethereum-mainnet")
            .expect("network")
            .chain_id_u64(),
        Some(1)
    );
    assert_eq!(
        validated
            .wallet("wallet_main")
            .expect("wallet")
            .subject
            .address_str(),
        "0x000000000000000000000000000000000000dead"
    );
    assert!(validated.symbol("eth.native.ethereum-mainnet").is_some());

    let portfolio = validated.into_config();
    assert_eq!(portfolio.wallets[0].wallet_id, "wallet_main");
}

#[test]
fn invalid_ref_detection_catches_cross_links() {
    let mut wallet_network = canonical_config_json();
    wallet_network["wallets"][0]["network_id"] = json!("unknown-network");
    assert_decode_error(
        &wallet_network,
        PortfolioConfigError::UnknownWalletNetwork {
            wallet_id: "wallet_treasury_eth".to_string(),
            network_id: "unknown-network".to_string(),
        },
    );

    let mut wallet_symbol = canonical_config_json();
    wallet_symbol["wallets"][0]["symbol_ids"][0] = json!("unknown-symbol");
    assert_decode_error(
        &wallet_symbol,
        PortfolioConfigError::UnknownWalletSymbol {
            wallet_id: "wallet_treasury_eth".to_string(),
            symbol_id: "unknown-symbol".to_string(),
        },
    );

    let mut duplicate_wallet_symbol = canonical_config_json();
    duplicate_wallet_symbol["wallets"][0]["symbol_ids"] =
        json!(["eth.native.ethereum-mainnet", "eth.native.ethereum-mainnet"]);
    assert_decode_error(
        &duplicate_wallet_symbol,
        PortfolioConfigError::DuplicateWalletSymbol {
            wallet_id: "wallet_treasury_eth".to_string(),
            symbol_id: "eth.native.ethereum-mainnet".to_string(),
        },
    );

    let mut symbol_network = canonical_config_json();
    symbol_network["symbol_configs"][0]["network_id"] = json!("unknown-network");
    assert_decode_error(
        &symbol_network,
        PortfolioConfigError::UnknownSymbolNetwork {
            symbol_id: "eth.native.ethereum-mainnet".to_string(),
            network_id: "unknown-network".to_string(),
        },
    );

    let mut priced_symbol = canonical_config_json();
    priced_symbol["symbol_configs"][1]["valuation"]["quotes"][0]["priced_symbol_id"] =
        json!("unknown-symbol");
    assert_decode_error(
        &priced_symbol,
        PortfolioConfigError::UnknownPricedSymbol {
            symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
            quote: QuoteCode::Usd,
            priced_symbol_id: "unknown-symbol".to_string(),
        },
    );

    let mut underlying_symbol = canonical_config_json();
    underlying_symbol["symbol_configs"][1]["underlying_symbol_id"] = json!("unknown-symbol");
    assert_decode_error(
        &underlying_symbol,
        PortfolioConfigError::UnknownUnderlyingSymbol {
            symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
            underlying_symbol_id: "unknown-symbol".to_string(),
        },
    );
}

#[test]
fn typed_metadata_rejects_non_string_values() {
    let mut portfolio_metadata = canonical_config_json();
    portfolio_metadata["metadata"] = json!({"threshold": 1.5});
    assert!(matches!(
        decode_portfolio_config(&portfolio_metadata),
        Err(PortfolioConfigError::Decode(_))
    ));

    let mut float_decimals = canonical_config_json();
    float_decimals["symbol_configs"][0]["decimals"] = json!(18.5);
    assert!(matches!(
        decode_portfolio_config(&float_decimals),
        Err(PortfolioConfigError::Decode(_))
    ));
}

#[test]
fn typed_metadata_rejects_secret_markers() {
    let mut portfolio_metadata = canonical_config_json();
    portfolio_metadata["metadata"] = json!({"mnemonic": "redacted"});
    assert_decode_rejects_public_metadata(&portfolio_metadata, "mnemonic");

    let mut network_metadata = canonical_config_json();
    network_metadata["networks"][0]["metadata"] = json!({"label": "password=redacted"});
    assert_decode_rejects_public_metadata(&network_metadata, "label");

    let mut wallet_metadata = canonical_config_json();
    wallet_metadata["wallets"][0]["metadata"] = json!({"api_key": "redacted"});
    assert_decode_rejects_public_metadata(&wallet_metadata, "api_key");

    let mut symbol_metadata = canonical_config_json();
    symbol_metadata["symbol_configs"][0]["metadata"] = json!({"note": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"});
    assert_decode_rejects_public_metadata(&symbol_metadata, "note");
}

fn assert_decode_rejects_public_metadata(value: &Value, expected_key: &str) {
    let error = decode_portfolio_config(value).expect_err("metadata must fail");
    let PortfolioConfigError::Decode(message) = error else {
        panic!("expected decode failure, got {error}");
    };
    assert!(
        message.contains(expected_key),
        "decode message `{message}` did not include `{expected_key}`"
    );
}

fn assert_decode_error(value: &Value, expected: PortfolioConfigError) {
    assert_eq!(decode_portfolio_config(value).unwrap_err(), expected);
}

#[test]
fn validation_rejects_non_normalized_addresses() {
    let mut wallet_address = canonical_config_json();
    wallet_address["wallets"][0]["subject"]["address"] =
        json!("0x000000000000000000000000000000000000DEAD");
    let err = decode_portfolio_config(&wallet_address).unwrap_err();
    assert!(matches!(err, PortfolioConfigError::Decode(message)
        if message.contains("normalized EVM address")));

    let mut token_address = canonical_config_json();
    token_address["symbol_configs"][1]["balance_reader"]["token_address"] =
        json!("0x000000000000000000000000000000000000000A");
    let err = decode_portfolio_config(&token_address).unwrap_err();
    assert!(matches!(err, PortfolioConfigError::Decode(message)
        if message.contains("normalized EVM address")));
}

#[test]
fn validation_rejects_quote_route_mismatches() {
    let mut duplicate_quote_codes = canonical_config_json();
    duplicate_quote_codes["quote_codes"] = json!(["USD", "USD"]);
    assert_decode_error(
        &duplicate_quote_codes,
        PortfolioConfigError::DuplicateQuoteCode {
            quote: QuoteCode::Usd,
        },
    );

    let mut missing_quote = canonical_config_json();
    missing_quote["symbol_configs"][0]["valuation"]["quotes"] = json!([
        {
            "quote": "USD",
            "priced_symbol_id": "eth.native.ethereum-mainnet",
            "unit_price_dec": "1.0"
        }
    ]);
    assert_decode_error(
        &missing_quote,
        PortfolioConfigError::MissingValuationQuote {
            symbol_id: "eth.native.ethereum-mainnet".to_string(),
            quote: QuoteCode::Btc,
        },
    );

    let mut unexpected_quote = canonical_config_json();
    unexpected_quote["quote_codes"] = json!(["USD"]);
    assert_decode_error(
        &unexpected_quote,
        PortfolioConfigError::UnexpectedValuationQuote {
            symbol_id: "eth.native.ethereum-mainnet".to_string(),
            quote: QuoteCode::Btc,
        },
    );
}

#[test]
fn normalization_sorts_config_and_runtime_outputs() {
    let mut cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");
    cfg.quote_codes.reverse();
    cfg.networks.reverse();
    cfg.wallets.reverse();
    cfg.wallets[0].symbol_ids.reverse();
    cfg.symbol_configs.reverse();
    cfg.symbol_configs[0].valuation.quotes.reverse();
    cfg.normalize();

    assert_eq!(cfg.quote_codes, vec![QuoteCode::Btc, QuoteCode::Usd]);
    assert_eq!(
        cfg.networks
            .iter()
            .map(|network| network.network_id().as_str())
            .collect::<Vec<_>>(),
        vec!["arbitrum-mainnet", "ethereum-mainnet"]
    );
    assert_eq!(
        cfg.wallets
            .iter()
            .map(|wallet| wallet.wallet_id.as_str())
            .collect::<Vec<_>>(),
        vec!["wallet_ops_arb", "wallet_treasury_eth"]
    );
    assert_eq!(
        cfg.wallets[1]
            .symbol_ids
            .iter()
            .map(|symbol_id| symbol_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "eth.native.ethereum-mainnet",
            "usdc.wallet.ethereum-mainnet"
        ]
    );
    assert_eq!(
        cfg.symbol_configs
            .iter()
            .map(|symbol| symbol.symbol_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "eth.native.arbitrum-mainnet",
            "eth.native.ethereum-mainnet",
            "usdc.wallet.ethereum-mainnet",
        ]
    );
    assert_eq!(
        cfg.symbol_configs[1]
            .valuation
            .quotes
            .iter()
            .map(|quote| quote.quote)
            .collect::<Vec<_>>(),
        vec![QuoteCode::Btc, QuoteCode::Usd]
    );

    let mut snapshot = PortfolioSnapshot {
        schema_version: 2,
        portfolio_id: "portfolio_main".to_string(),
        generated_at_ms: 1,
        network_pins: vec![
            NetworkPin {
                network_id: "ethereum-mainnet".to_string(),
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 10,
                    block_hash: EVM_HASH.to_owned(),
                },
            },
            NetworkPin {
                network_id: "arbitrum-mainnet".to_string(),
                anchor: ExecutionAnchor::Evm {
                    chain_id: 42161,
                    block_number: 20,
                    block_hash: EVM_HASH.to_owned(),
                },
            },
        ],
        wallets: vec![
            WalletSnapshot {
                wallet_id: "wallet_treasury_eth".to_string(),
                address: "0x000000000000000000000000000000000000dead".to_string(),
                subject_kind: WalletSubjectKind::EvmAddress,
                network_id: "ethereum-mainnet".to_string(),
                observations: vec![
                    observation(
                        "wallet_treasury_eth",
                        "usdc.wallet.ethereum-mainnet",
                        vec![QuoteCode::Usd, QuoteCode::Btc],
                    ),
                    observation(
                        "wallet_treasury_eth",
                        "eth.native.ethereum-mainnet",
                        vec![QuoteCode::Usd, QuoteCode::Btc],
                    ),
                ],
            },
            WalletSnapshot {
                wallet_id: "wallet_ops_arb".to_string(),
                address: "0x000000000000000000000000000000000000beef".to_string(),
                subject_kind: WalletSubjectKind::EvmAddress,
                network_id: "arbitrum-mainnet".to_string(),
                observations: vec![observation(
                    "wallet_ops_arb",
                    "eth.native.arbitrum-mainnet",
                    vec![QuoteCode::Usd, QuoteCode::Btc],
                )],
            },
        ],
        symbol_configs: cfg.symbol_configs.clone(),
    };
    snapshot.network_pins.reverse();
    snapshot.wallets.reverse();
    snapshot.wallets[0].observations.reverse();
    snapshot.normalize();

    assert_eq!(
        snapshot
            .network_pins
            .iter()
            .map(|pin| pin.network_id.as_str())
            .collect::<Vec<_>>(),
        vec!["arbitrum-mainnet", "ethereum-mainnet"]
    );
    assert_eq!(
        snapshot
            .wallets
            .iter()
            .map(|wallet| wallet.wallet_id.as_str())
            .collect::<Vec<_>>(),
        vec!["wallet_ops_arb", "wallet_treasury_eth"]
    );
    assert_eq!(
        snapshot.wallets[1]
            .observations
            .iter()
            .map(|observation| observation.symbol_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "eth.native.ethereum-mainnet",
            "usdc.wallet.ethereum-mainnet",
        ]
    );
}

fn canonical_config_json() -> Value {
    json!({
        "portfolio_id": "portfolio_main",
        "quote_codes": ["USD", "BTC"],
        "networks": [
            {
                "network_id": "ethereum-mainnet",
                "family": "evm",
                "chain_id": 1,
                "metadata": {}
            },
            {
                "network_id": "arbitrum-mainnet",
                "family": "evm",
                "chain_id": 42161,
                "metadata": {}
            }
        ],
        "wallets": [
            {
                "wallet_id": "wallet_treasury_eth",
                "subject": {
                    "kind": "evm_address",
                    "address": "0x000000000000000000000000000000000000dead"
                },
                "implementation": {
                    "kind": "address_only"
                },
                "network_id": "ethereum-mainnet",
                "symbol_ids": [
                    "eth.native.ethereum-mainnet",
                    "usdc.wallet.ethereum-mainnet"
                ],
                "metadata": {}
            },
            {
                "wallet_id": "wallet_ops_arb",
                "subject": {
                    "kind": "evm_address",
                    "address": "0x000000000000000000000000000000000000beef"
                },
                "implementation": {
                    "kind": "address_only"
                },
                "network_id": "arbitrum-mainnet",
                "symbol_ids": [
                    "eth.native.arbitrum-mainnet"
                ],
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
                            "unit_price_dec": "1800.0"
                        },
                        {
                            "quote": "BTC",
                            "priced_symbol_id": "eth.native.ethereum-mainnet",
                            "unit_price_dec": "0.05"
                        }
                    ]
                },
                "decimals": 18,
                "underlying_symbol_id": null,
                "metadata": {}
            },
            {
                "symbol_id": "usdc.wallet.ethereum-mainnet",
                "display_symbol": "USDC",
                "kind": "erc20_balance",
                "role": "asset",
                "network_id": "ethereum-mainnet",
                "protocol": null,
                "balance_reader": {
                    "kind": "erc20_balance",
                    "token_address": "0x0000000000000000000000000000000000000001"
                },
                "valuation": {
                    "quotes": [
                        {
                            "quote": "USD",
                            "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                            "unit_price_dec": "1.0"
                        },
                        {
                            "quote": "BTC",
                            "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                            "unit_price_dec": "0.00001"
                        }
                    ]
                },
                "decimals": 6,
                "underlying_symbol_id": null,
                "metadata": {}
            },
            {
                "symbol_id": "eth.native.arbitrum-mainnet",
                "display_symbol": "ETH",
                "kind": "native_balance",
                "role": "native",
                "network_id": "arbitrum-mainnet",
                "protocol": null,
                "balance_reader": {
                    "kind": "native_balance"
                },
                "valuation": {
                    "quotes": [
                        {
                            "quote": "USD",
                            "priced_symbol_id": "eth.native.arbitrum-mainnet",
                            "unit_price_dec": "1800.0"
                        },
                        {
                            "quote": "BTC",
                            "priced_symbol_id": "eth.native.arbitrum-mainnet",
                            "unit_price_dec": "0.05"
                        }
                    ]
                },
                "decimals": 18,
                "underlying_symbol_id": null,
                "metadata": {}
            }
        ],
        "metadata": {}
    })
}

fn observation(wallet_id: &str, symbol_id: &str, value_order: Vec<QuoteCode>) -> Observation {
    Observation {
        wallet_id: wallet_id.to_string(),
        symbol_id: symbol_id.to_string(),
        display_symbol: Some("ETH".to_string()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: if symbol_id.contains("arbitrum") {
            "arbitrum-mainnet".to_string()
        } else {
            "ethereum-mainnet".to_string()
        },
        protocol: None,
        quantity: ObservationQuantity {
            raw_dec: "1".to_string(),
            decimals: 18,
            amount_dec: "0.000000000000000001".to_string(),
        },
        values: value_order
            .into_iter()
            .map(|quote| ObservationValue {
                quote,
                priced_symbol_id: symbol_id.to_string(),
                value_dec: "1".to_string(),
                unit_price_dec: "1".to_string(),
            })
            .collect(),
        source: ObservationSource {
            balance_reader_kind: "native_balance".to_string(),
            network_id: if symbol_id.contains("arbitrum") {
                "arbitrum-mainnet".to_string()
            } else {
                "ethereum-mainnet".to_string()
            },
            anchor: ObservationAnchor::Evm {
                chain_id: if symbol_id.contains("arbitrum") {
                    42161
                } else {
                    1
                },
                block_number: 1,
                block_hash: EVM_HASH.to_owned(),
            },
        },
        coverage: "configured_only".to_string(),
        metadata: PublicMetadata::default(),
    }
}
