use super::*;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::symbol::{SymbolKind, SymbolValuationConfig};
use mfm_portfolio_model::wallet::WalletSubject;
use mfm_values::MfmConfig as _;

fn test_network(network_id: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network_id.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
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
        U256::from(1_000_000_000_000_000_000u128),
        18,
        10,
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
