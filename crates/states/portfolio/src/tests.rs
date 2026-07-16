use super::*;
use std::collections::BTreeMap;

use mfm_facts::FactContentIdentityEvidence;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, NetworkPin,
};
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, HoldingSourceConfig, Observation, ObservationAnchor,
    ObservationQuantity, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";

fn test_network(network_id: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network_id.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
        Some(18),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("valid network config")
}

fn test_wallet(network_id: &str) -> WalletConfig {
    WalletConfig {
        wallet_id: "wallet_main".parse().expect("valid wallet id"),
        subject: WalletSubject::new(EVM_ACCOUNT, WalletSubjectKind::EvmAddress)
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
        network_id: network_id.parse().expect("valid network id"),
        source: HoldingSourceConfig::Native,
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid priced symbol id"),
                unit_price_dec: "2.5".parse().expect("valid unit price"),
            }],
        },
        metadata: PublicMetadata::default(),
    }
}

fn sample_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![test_network("ethereum-mainnet", 1)],
        wallets: vec![test_wallet("ethereum-mainnet")],
        symbol_configs: vec![test_symbol("ethereum-mainnet")],
        metadata: PublicMetadata::default(),
    }
}

fn requirement() -> HoldingRequirementKey {
    HoldingRequirementKey {
        wallet_id: "wallet_main".to_owned(),
        symbol_id: "eth.native.ethereum-mainnet".to_owned(),
        network_id: "ethereum-mainnet".to_owned(),
    }
}

fn native_source() -> HoldingSourceKey {
    HoldingSourceKey::EvmNative {
        network_id: "ethereum-mainnet".to_owned(),
        chain_id: 1,
        account: EVM_ACCOUNT.to_owned(),
    }
}

fn identity_evidence() -> FactContentIdentityEvidence {
    serde_json::from_value(serde_json::json!({
        "fact_descriptor_hash": "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "subject_material_hash": "content:sha256-jcs-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "response_schema_id": "schema:mfm.portfolio.test.response:mfm.portfolio.test.v1:sha256-jcs-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "response_hash": "content:sha256-jcs-v1:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    }))
    .expect("identity evidence")
}

fn manifest_entry() -> HoldingManifestEntry {
    HoldingManifestEntry::new(requirement(), native_source()).expect("manifest entry")
}

fn collected_holding() -> CollectedHoldingReceipt {
    CollectedHoldingReceipt::new(
        requirement(),
        native_source(),
        ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 10,
            block_hash: EVM_HASH.to_owned(),
        },
        "configured_only".to_owned(),
        "ok".to_owned(),
        identity_evidence(),
    )
    .expect("collected holding")
}

fn receipt() -> PortfolioCollectionReceipt {
    PortfolioCollectionReceipt::new(
        &[manifest_entry()],
        vec![collected_holding()],
        vec![NetworkPin {
            network_id: "ethereum-mainnet".to_owned(),
            anchor: ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: EVM_HASH.to_owned(),
            },
        }],
    )
    .expect("collection receipt")
}

fn observation(raw_dec: &str) -> Observation {
    Observation {
        wallet_id: "wallet_main".to_owned(),
        symbol_id: "eth.native.ethereum-mainnet".to_owned(),
        display_symbol: Some("ETH".to_owned()),
        network_id: "ethereum-mainnet".to_owned(),
        quantity: ObservationQuantity {
            raw_dec: raw_dec.to_owned(),
            decimals: 18,
            amount_dec: if raw_dec == "0" {
                "0.000000000000000000".to_owned()
            } else {
                "1.000000000000000000".to_owned()
            },
        },
        values: Vec::new(),
        source: AnchoredHoldingSource {
            holding: HoldingSourceConfig::Native,
            anchor: ObservationAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: EVM_HASH.to_owned(),
            },
        },
        coverage: "configured_only".to_owned(),
        metadata: PublicMetadata::default(),
    }
}

#[test]
fn unknown_portfolio_config_fields_are_rejected_on_decode() {
    assert_unknown_field_rejected::<PortfolioConfig>(
        r#"{"unexpected_field":1}"#,
        "portfolio config",
    );
}

#[test]
fn output_projection_configs_carry_no_version_policy() {
    let mut snapshot_config =
        serde_json::to_value(AssembleSnapshotConfig::new(sample_portfolio()).expect("config"))
            .expect("snapshot config serializes");
    assert!(snapshot_config.get("snapshot_version").is_none());
    snapshot_config["snapshot_version"] = serde_json::json!(2);
    assert!(serde_json::from_value::<AssembleSnapshotConfig>(snapshot_config).is_err());

    let mut report_config =
        serde_json::to_value(ProjectReportConfig::default()).expect("report config serializes");
    assert_eq!(report_config, serde_json::json!({}));
    report_config["report_version"] = serde_json::json!(2);
    assert!(serde_json::from_value::<ProjectReportConfig>(report_config).is_err());
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
fn collection_receipt_rejects_incomplete_or_aliased_logical_demand() {
    assert_eq!(
        manifest_identity(&[]).expect_err("empty demand").code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    let first = manifest_entry();
    let second = HoldingManifestEntry::new(
        HoldingRequirementKey {
            wallet_id: "wallet_other".to_owned(),
            ..requirement()
        },
        native_source(),
    )
    .expect("aliased source is individually well formed");
    assert_eq!(
        manifest_identity(&[first, second])
            .expect_err("aliased source")
            .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    let holding = collected_holding();
    assert_eq!(
        PortfolioCollectionReceipt::new(
            &[manifest_entry()],
            vec![holding.clone(), holding],
            receipt().network_anchors().to_vec(),
        )
        .expect_err("duplicate completed holding")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    let unexpected_source = HoldingSourceKey::EvmErc20 {
        network_id: "ethereum-mainnet".to_owned(),
        chain_id: 1,
        contract_address: "0x0000000000000000000000000000000000000001".to_owned(),
        account: EVM_ACCOUNT.to_owned(),
    };
    let unexpected = CollectedHoldingReceipt::new(
        requirement(),
        unexpected_source,
        ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 10,
            block_hash: EVM_HASH.to_owned(),
        },
        "complete_at_anchor".to_owned(),
        "ok".to_owned(),
        identity_evidence(),
    )
    .expect("well formed unexpected source");
    assert_eq!(
        PortfolioCollectionReceipt::new(
            &[manifest_entry()],
            vec![unexpected],
            receipt().network_anchors().to_vec(),
        )
        .expect_err("unexpected source")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );
}

#[test]
fn collection_receipt_rejects_source_status_and_anchor_mismatches() {
    let wrong_network = HoldingSourceKey::EvmNative {
        network_id: "other-network".to_owned(),
        chain_id: 1,
        account: EVM_ACCOUNT.to_owned(),
    };
    assert_eq!(
        HoldingManifestEntry::new(requirement(), wrong_network)
            .expect_err("source network mismatch")
            .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    assert_eq!(
        CollectedHoldingReceipt::new(
            requirement(),
            native_source(),
            ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: EVM_HASH.to_owned(),
            },
            "truncated".to_owned(),
            "ok".to_owned(),
            identity_evidence(),
        )
        .expect_err("inadmissible coverage")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    assert_eq!(
        CollectedHoldingReceipt::new(
            requirement(),
            native_source(),
            ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: "not-a-block-hash".to_owned(),
            },
            "configured_only".to_owned(),
            "ok".to_owned(),
            identity_evidence(),
        )
        .expect_err("malformed EVM anchor")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    let malformed_evidence = serde_json::from_value::<FactContentIdentityEvidence>(
        serde_json::json!({
            "fact_descriptor_hash": "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "subject_material_hash": "content:sha256-jcs-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "response_schema_id": "schema:mfm.portfolio.test.response:mfm.portfolio.test.v1:sha256-jcs-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "response_hash": "not-a-content-digest"
        }),
    );
    assert!(
        malformed_evidence.is_err(),
        "malformed opaque fact identity evidence must be rejected on decode"
    );

    assert_eq!(
        CollectedHoldingReceipt::new(
            requirement(),
            native_source(),
            ExecutionAnchor::Bitcoin {
                height: 10,
                block_hash: "aa".repeat(32),
            },
            "configured_only".to_owned(),
            "ok".to_owned(),
            identity_evidence(),
        )
        .expect_err("anchor family mismatch")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );

    assert_eq!(
        PortfolioCollectionReceipt::new(
            &[manifest_entry()],
            vec![collected_holding()],
            vec![NetworkPin {
                network_id: "ethereum-mainnet".to_owned(),
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 11,
                    block_hash: EVM_HASH.to_owned(),
                },
            }],
        )
        .expect_err("anchor number mismatch")
        .code,
        PortfolioHoldingErrorCode::ReceiptMismatch
    );
}

#[test]
fn collection_receipt_decode_rejects_manifest_identity_tampering() {
    let mut value = serde_json::to_value(receipt()).expect("receipt value");
    value["manifest_identity"] = serde_json::json!("sha256-jcs-v1:tampered");
    assert!(
        serde_json::from_value::<PortfolioCollectionReceipt>(value).is_err(),
        "receipt decode must rebuild and compare its logical manifest identity"
    );
}

#[test]
fn selection_config_closes_receipt_policy_and_candidate_bound() {
    let config = SelectHoldingsConfig::new(sample_portfolio()).expect("config");
    assert_eq!(
        config.selection_policy_id(),
        PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID
    );
    assert_eq!(config.candidate_bound(), 10);
    assert_eq!(config.candidate_scan_limit(), 11);

    let mut value = serde_json::to_value(config).expect("config value");
    value["candidate_scan_limit"] = serde_json::json!(12);
    let tampered: SelectHoldingsConfig = serde_json::from_value(value).expect("decode config");
    assert!(mfm_program::ValidatedConfig::new(tampered).is_err());
}

#[test]
fn fixed_price_selection_assembles_direct_totals_and_receipt_pins() {
    let portfolio = sample_portfolio();
    let receipt = receipt();
    validate_receipt_against_portfolio(&receipt, &portfolio).expect("receipt matches portfolio");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.clone()).expect("valuation config"),
    )
    .expect("valuations");

    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(portfolio).expect("assemble config"),
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: vec![observation("1000000000000000000")],
            },
            valuations,
            receipt: receipt.clone(),
        },
    )
    .expect("snapshot");
    assert_eq!(snapshot.network_pins, receipt.network_anchors());
    assert_eq!(snapshot.schema_version, PortfolioSnapshot::SCHEMA_VERSION);

    let mut unsupported_snapshot = snapshot.clone();
    unsupported_snapshot.schema_version = 2;
    assert!(project_report_from_snapshot(unsupported_snapshot).is_err());

    let report = project_report_from_snapshot(snapshot).expect("report");
    assert_eq!(report.schema_version, PortfolioReport::SCHEMA_VERSION);
    assert_eq!(report.totals_by_quote[0].total_value_dec, "2.5");
    assert_eq!(
        report.wallet_summaries[0].totals_by_quote[0].total_value_dec,
        "2.5"
    );
}

#[test]
fn direct_total_reducer_preserves_configured_zero_rows() {
    let portfolio = sample_portfolio();
    let receipt = receipt();
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.clone()).expect("valuation config"),
    )
    .expect("valuations");
    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(portfolio).expect("assemble config"),
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: vec![observation("0")],
            },
            valuations,
            receipt,
        },
    )
    .expect("snapshot");

    let report = project_report_from_snapshot(snapshot).expect("report");
    assert_eq!(report.wallet_summaries[0].totals_by_quote.len(), 1);
    assert_eq!(
        report.wallet_summaries[0].totals_by_quote[0].total_value_dec,
        "0"
    );
    assert_eq!(report.totals_by_quote[0].total_value_dec, "0");
}

#[test]
fn assemble_hard_fails_on_missing_or_substituted_receipt_observations() {
    let portfolio = sample_portfolio();
    let config = AssembleSnapshotConfig::new(portfolio.clone()).expect("assemble config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio).expect("valuation config"),
    )
    .expect("valuations");
    let receipt = receipt();

    let missing = assemble_snapshot(
        &config,
        AssembleSnapshotInput {
            subjects: subjects.clone(),
            holdings: SelectedHoldings {
                observations: Vec::new(),
            },
            valuations: valuations.clone(),
            receipt: receipt.clone(),
        },
    )
    .expect_err("missing holding must prevent root output");
    assert!(missing.to_string().contains("receipt_mismatch"));

    let mut substituted = observation("1");
    substituted.source.anchor = ObservationAnchor::Evm {
        chain_id: 1,
        block_number: 11,
        block_hash: EVM_HASH.to_owned(),
    };
    let mismatch = assemble_snapshot(
        &config,
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: vec![substituted],
            },
            valuations,
            receipt,
        },
    )
    .expect_err("anchor substitution must prevent root output");
    assert!(mismatch.to_string().contains("receipt_mismatch"));
}
