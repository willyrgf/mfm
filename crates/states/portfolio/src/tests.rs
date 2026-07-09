use super::*;
use mfm_portfolio_model::aave::{
    AaveMarketConfig, AaveProtocolPositionConfig, AaveReserveConfig, AaveReservePositionConfig,
    AAVE_V3_PROTOCOL_ID, AAVE_V3_READER_RESERVE_POSITION,
};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::NetworkFamilyConfig;
use mfm_portfolio_model::symbol::{SymbolKind, SymbolValuationConfig};
use mfm_portfolio_model::wallet::WalletSubject;
use mfm_values::MfmConfig as _;

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

fn test_network(network_id: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network_id.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
        None,
        "shared".to_owned(),
        BTreeMap::new(),
    )
    .expect("valid network config")
}

fn test_wallet(network_id: &str) -> WalletConfig {
    WalletConfig {
        wallet_id: "wallet_main".parse().expect("valid wallet id"),
        subject: WalletSubject::new(
            "0x000000000000000000000000000000000000dead",
            WalletSubjectKind::EvmAddress,
        )
        .expect("valid wallet subject"),
        network_id: network_id.parse().expect("valid network id"),
        implementation: WalletImplementationConfig::AddressOnly {},
        symbol_ids: vec!["eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id")],
        metadata: PublicMetadata::default(),
    }
}

fn test_symbol(network_id: &str) -> SymbolConfig {
    SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: network_id.parse().expect("valid network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid priced symbol id"),
                reader: ValuationReaderConfig::FixedUnitPrice {
                    unit_price_dec: "2.5".parse().expect("valid unit price"),
                },
            }],
        },
        decimals: Some(18),
        underlying_symbol_id: None,
        metadata: PublicMetadata::default(),
    }
}

#[test]
fn unknown_portfolio_config_fields_are_rejected_on_decode() {
    assert_unknown_field_rejected::<PortfolioWorkflowConfig>(
        r#"{"unexpected_field":1}"#,
        "workflow config",
    );
    assert_unknown_field_rejected::<PinViewsConfig>(
        r#"{"pinned_networks":[],"unexpected_field":1}"#,
        "pin views config",
    );
    assert_unknown_field_rejected::<MergeObservationsConfig>(
        r#"{"unexpected_field":1}"#,
        "merge observations config",
    );
}

fn assert_unknown_field_rejected<T>(input: &str, label: &str)
where
    T: serde::de::DeserializeOwned,
{
    let error = match serde_json::from_str::<T>(input) {
        Ok(_) => panic!("{label} accepted unknown field"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("unknown field"),
        "{label}: {error}"
    );
}

#[test]
fn observe_batch_mfm_config_validation_rejects_network_mismatch() {
    let config = ObserveBatchConfig {
        wallet: test_wallet("ethereum-mainnet"),
        symbol: test_symbol("ethereum-mainnet"),
        network: test_network("ethereum-goerli", 5),
    };

    let error = config.validate().expect_err("network mismatch must fail");
    assert!(
        error
            .message()
            .contains("did not match observation network"),
        "{error}"
    );
}

#[test]
fn observation_read_intent_classifies_erc20_balance() {
    let network = test_evm_network("ethereum-mainnet", 1);
    let wallet = test_evm_wallet(
        "wallet_main",
        network.network_id().as_str(),
        vec!["usdc.ethereum-mainnet"],
    );
    let symbol = SymbolConfig {
        symbol_id: "usdc.ethereum-mainnet".parse().expect("symbol id"),
        display_symbol: Some("USDC".to_owned()),
        kind: SymbolKind::Erc20Balance,
        role: SymbolRole::Asset,
        network_id: network.network_id().as_str().parse().expect("network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::Erc20Balance {
            token_address: "0x0000000000000000000000000000000000000001"
                .parse()
                .expect("token address"),
        },
        valuation: fixed_usd_valuation("usdc.ethereum-mainnet"),
        decimals: None,
        underlying_symbol_id: None,
        metadata: PublicMetadata::default(),
    };
    let config = ObserveBatchConfig::new(wallet, symbol, network).expect("observe config");

    assert_eq!(
        observe_batch_network_read_intent(&config),
        Some(PortfolioNetworkReadIntent::Evm {
            network_id: "ethereum-mainnet".to_owned(),
            chain_id: 1,
        })
    );
    assert_eq!(
        observe_batch_read_intent(
            &config,
            &ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 123,
                block_hash: EVM_HASH.to_owned(),
            },
        )
        .expect("read intent"),
        PortfolioBalanceReadIntent::Erc20Balance {
            network_id: "ethereum-mainnet".to_owned(),
            chain_id: 1,
            account: "0x000000000000000000000000000000000000dead".to_owned(),
            token_address: "0x0000000000000000000000000000000000000001".to_owned(),
            block_number: 123,
            block_hash: EVM_HASH.to_owned(),
            decimals: None,
            anchor: ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 123,
                block_hash: EVM_HASH.to_owned(),
            },
        }
    );
}

#[test]
fn observation_read_intent_classifies_bitcoin_native_balance() {
    let network = test_bitcoin_network();
    let wallet = test_bitcoin_wallet(
        "wallet_btc_mainnet",
        network.network_id().as_str(),
        vec!["btc.native.bitcoin-mainnet"],
    );
    let symbol = SymbolConfig {
        symbol_id: "btc.native.bitcoin-mainnet".parse().expect("symbol id"),
        display_symbol: Some("BTC".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: network.network_id().as_str().parse().expect("network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: fixed_usd_valuation("btc.native.bitcoin-mainnet"),
        decimals: None,
        underlying_symbol_id: None,
        metadata: PublicMetadata::default(),
    };
    let config = ObserveBatchConfig::new(wallet, symbol, network).expect("observe config");
    let anchor = ExecutionAnchor::Bitcoin {
        height: 850_000,
        block_hash: "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc".to_owned(),
    };

    assert_eq!(
        observe_batch_network_read_intent(&config),
        Some(PortfolioNetworkReadIntent::Bitcoin {
            network_id: "bitcoin-mainnet".to_owned(),
            source_identity: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
        })
    );
    assert_eq!(
        observe_batch_read_intent(&config, &anchor).expect("read intent"),
        PortfolioBalanceReadIntent::BitcoinNativeBalance {
            network_id: "bitcoin-mainnet".to_owned(),
            source_identity: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
            address: "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw".to_owned(),
            anchor,
            decimals: 8,
        }
    );
}

#[test]
fn protocol_position_classifies_as_unsupported_without_network_requirement() {
    let network = test_evm_network("ethereum-mainnet", 1);
    let wallet = test_evm_wallet(
        "wallet_main",
        network.network_id().as_str(),
        vec!["aave.weth.ethereum-mainnet"],
    );
    let symbol = aave_reserve_symbol(network.network_id().as_str());
    let config = ObserveBatchConfig::new(wallet, symbol, network).expect("observe config");

    assert_eq!(observe_batch_network_read_intent(&config), None);
    let error = observe_batch_read_intent(
        &config,
        &ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 123,
            block_hash: EVM_HASH.to_owned(),
        },
    )
    .expect_err("protocol positions are unsupported by the typed read adapter");
    assert_eq!(error.code, "unsupported_balance_reader");
}

#[test]
fn fixed_price_observation_projects_report_totals() {
    let network = test_network("ethereum-mainnet", 1);
    let wallet = test_wallet(network.network_id().as_str());
    let symbol = test_symbol(network.network_id().as_str());
    let input = ObserveBatchInput {
        subjects: ResolvedSubjects {
            subjects: vec![ResolvedSubject {
                wallet_id: wallet.wallet_id.to_string(),
                address: wallet.subject.address_str().to_owned(),
                subject_kind: wallet.subject.kind(),
                network_id: wallet.network_id.to_string(),
                implementation_kind: "address_only".to_owned(),
            }],
        },
        views: PinnedViews {
            views: vec![PinnedView {
                network_id: network.network_id().to_string(),
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 10,
                    block_hash: EVM_HASH.to_owned(),
                },
            }],
        },
        valuations: ResolvedValuations {
            valuations: vec![ResolvedValuation {
                symbol_id: symbol.symbol_id.to_string(),
                quote: QuoteCode::Usd,
                priced_symbol_id: symbol.symbol_id.to_string(),
                unit_price_dec: "2.5".to_owned(),
                valuation_reader_kind: "fixed_unit_price".to_owned(),
                source_refs: Vec::new(),
            }],
            errors: Vec::new(),
        },
    };
    let batch = observation_batch_from_raw_balance(
        &ObserveBatchConfig::new(wallet.clone(), symbol.clone(), network.clone())
            .expect("observe config"),
        &input,
        RawBalanceObservation::new(U256::from(1_000_000_000_000_000_000u128), 18, None),
    );
    assert_eq!(
        batch.observations[0].values[0].value_dec,
        "2.500000000000000000"
    );

    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(
            2,
            PortfolioConfig {
                portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
                quote_codes: vec![QuoteCode::Usd],
                networks: vec![network],
                wallets: vec![wallet],
                symbol_configs: vec![symbol],
                metadata: PublicMetadata::default(),
            },
        )
        .expect("assemble config"),
        AssembleSnapshotInput {
            subjects: input.subjects,
            views: input.views,
            observations: MergedObservations {
                observations: batch.observations,
                errors: Vec::new(),
            },
        },
        42,
    );
    let report = project_report_from_snapshot(snapshot, 2).expect("report");
    assert_eq!(
        report.totals_by_quote[0].assets_value_dec,
        "2.500000000000000000"
    );
    assert_eq!(
        report.totals_by_quote[0].net_value_dec,
        "2.500000000000000000"
    );
}

fn test_evm_network(network_id: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network_id.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
        None,
        "shared".to_owned(),
        BTreeMap::new(),
    )
    .expect("valid EVM network")
}

fn test_bitcoin_network() -> NetworkConfig {
    NetworkConfig::new(
        "bitcoin-mainnet".to_owned(),
        NetworkFamilyConfig::Bitcoin,
        None,
        Some("main".to_owned()),
        "bitcoin-mainnet".to_owned(),
        BTreeMap::new(),
    )
    .expect("valid Bitcoin network")
}

fn test_evm_wallet(wallet_id: &str, network_id: &str, symbol_ids: Vec<&str>) -> WalletConfig {
    test_wallet_with_subject(
        wallet_id,
        "0x000000000000000000000000000000000000dead",
        WalletSubjectKind::EvmAddress,
        network_id,
        symbol_ids,
    )
}

fn test_bitcoin_wallet(wallet_id: &str, network_id: &str, symbol_ids: Vec<&str>) -> WalletConfig {
    test_wallet_with_subject(
        wallet_id,
        "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw",
        WalletSubjectKind::BitcoinAddress,
        network_id,
        symbol_ids,
    )
}

fn test_wallet_with_subject(
    wallet_id: &str,
    address: &str,
    subject_kind: WalletSubjectKind,
    network_id: &str,
    symbol_ids: Vec<&str>,
) -> WalletConfig {
    WalletConfig {
        wallet_id: wallet_id.parse().expect("wallet id"),
        subject: WalletSubject::new(address, subject_kind).expect("wallet subject"),
        network_id: network_id.parse().expect("network id"),
        implementation: WalletImplementationConfig::AddressOnly {},
        symbol_ids: symbol_ids
            .into_iter()
            .map(|symbol_id| symbol_id.parse().expect("symbol id"))
            .collect(),
        metadata: PublicMetadata::default(),
    }
}

fn fixed_usd_valuation(symbol_id: &str) -> SymbolValuationConfig {
    SymbolValuationConfig {
        quotes: vec![QuoteValuationConfig {
            quote: QuoteCode::Usd,
            priced_symbol_id: symbol_id.parse().expect("priced symbol id"),
            reader: ValuationReaderConfig::FixedUnitPrice {
                unit_price_dec: "1.0".parse().expect("unit price"),
            },
        }],
    }
}

fn aave_reserve_symbol(network_id: &str) -> SymbolConfig {
    let market = AaveMarketConfig {
        market_id: "aave_v3_ethereum".parse().expect("market id"),
        network_id: network_id.parse().expect("network id"),
        chain_id: 1,
        pool_address: "0x0000000000000000000000000000000000000002"
            .parse()
            .expect("pool address"),
        reserves: vec![AaveReserveConfig {
            reserve_id: "weth".parse().expect("reserve id"),
            reserve_index: 0,
            underlying_token_address: "0x0000000000000000000000000000000000000003"
                .parse()
                .expect("underlying token"),
            a_token_address: "0x0000000000000000000000000000000000000004"
                .parse()
                .expect("a token"),
            variable_debt_token_address: None,
            stable_debt_token_address: None,
            metadata: PublicMetadata::default(),
        }],
        metadata: PublicMetadata::default(),
    };
    SymbolConfig {
        symbol_id: "aave.weth.ethereum-mainnet".parse().expect("symbol id"),
        display_symbol: Some("aWETH".to_owned()),
        kind: SymbolKind::ProtocolPosition,
        role: SymbolRole::Collateral,
        network_id: network_id.parse().expect("network id"),
        protocol: Some(AAVE_V3_PROTOCOL_ID.parse().expect("protocol id")),
        balance_reader: BalanceReaderConfig::ProtocolPosition {
            protocol: AAVE_V3_PROTOCOL_ID.parse().expect("reader protocol"),
            reader: AAVE_V3_READER_RESERVE_POSITION.parse().expect("reader id"),
            config: AaveProtocolPositionConfig::ReservePosition(AaveReservePositionConfig {
                market,
                reserve_id: "weth".parse().expect("reserve id"),
                use_as_collateral_required: Some(true),
            }),
        },
        valuation: fixed_usd_valuation("eth.native.ethereum-mainnet"),
        decimals: Some(18),
        underlying_symbol_id: Some("eth.native.ethereum-mainnet".parse().expect("underlying")),
        metadata: PublicMetadata::default(),
    }
}
