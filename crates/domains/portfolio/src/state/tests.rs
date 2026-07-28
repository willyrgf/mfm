use alloy_primitives::{B256, U256};
use mfm_evm::{
    EvmAnchoredSource, EvmBalanceAsset, EvmBalanceCollection, EvmBlockAnchor, EvmCheckedSource,
    EvmRoutingGenerationRef,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId};
use mfm_program::State;
use serde_json::json;

use super::*;
use crate::decode_portfolio_config;

const ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

fn generation(seed: &[u8]) -> EvmRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.test.evm-routing",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"routing schema"),
    )
    .expect("schema");
    EvmRoutingGenerationRef::from_content_ref(
        ContentRef::new(
            schema,
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(seed)),
        )
        .expect("content ref"),
    )
    .expect("generation")
}

fn evm_portfolio() -> PortfolioConfig {
    decode_portfolio_config(&json!({
        "portfolio_id": "portfolio-main",
        "quote_codes": ["USD"],
        "networks": [{
            "family": "evm",
            "network_id": "ethereum-mainnet",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {}
        }],
        "wallets": [{
            "wallet_id": "wallet-main",
            "subject": {"kind": "evm_address", "address": ACCOUNT},
            "network_id": "ethereum-mainnet",
            "implementation": {"kind": "address_only"},
            "symbol_ids": ["eth-native", "usdc-token"],
            "metadata": {}
        }],
        "symbol_configs": [
            {
                "symbol_id": "eth-native",
                "display_symbol": "ETH",
                "network_id": "ethereum-mainnet",
                "source": {"kind": "native"},
                "valuation": {"quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": "eth-native",
                    "unit_price_dec": "2000"
                }]},
                "metadata": {}
            },
            {
                "symbol_id": "usdc-token",
                "display_symbol": "USDC",
                "network_id": "ethereum-mainnet",
                "source": {
                    "kind": "erc20",
                    "contract_address": TOKEN
                },
                "valuation": {"quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": "usdc-token",
                    "unit_price_dec": "1"
                }]},
                "metadata": {}
            }
        ],
        "metadata": {}
    }))
    .expect("portfolio")
}

fn selection_input() -> PortfolioSnapshotSelectionInput {
    PortfolioSnapshotSelectionInput::new(
        PortfolioSnapshotSelector::new(crate::PortfolioId::new("portfolio-main").expect("target")),
        evm_portfolio(),
        PortfolioRoutingManifest::new(vec![EvmRoutingBinding::new(
            "ethereum-mainnet",
            generation(b"generation-a"),
        )
        .expect("binding")])
        .expect("routing manifest"),
    )
}

fn selection() -> ValidatedPortfolioSnapshotSelection {
    validate_snapshot_selection(&selection_input()).expect("selection")
}

fn collection(selection: &ValidatedPortfolioSnapshotSelection) -> EvmBalanceCollection {
    let config = selection.positions()[0]
        .collection_config()
        .expect("collection");
    let checked = EvmCheckedSource::new(
        config.binding().clone(),
        "mfm.evm.json-rpc",
        "mfm.evm.json-rpc.v1",
    )
    .expect("checked source");
    let anchored = EvmAnchoredSource::new(
        checked,
        EvmBlockAnchor::new(U256::from(42), B256::from([0x11; 32])),
    )
    .expect("anchor");
    let balances = config
        .sources()
        .iter()
        .map(|source| {
            let (raw_units, decimals) = match source.asset() {
                EvmBalanceAsset::Native => ("2000000000000000000", 18),
                EvmBalanceAsset::Erc20 { .. } => ("3500000", 6),
            };
            json!({
                "source": source,
                "raw_units": raw_units,
                "decimals": decimals,
            })
        })
        .collect::<Vec<_>>();
    serde_json::from_value(json!({
        "source": anchored,
        "balances": balances,
    }))
    .expect("collection")
}

#[test]
fn validator_requires_exact_demanded_generation_set() {
    let mut missing = selection_input();
    missing.routing_manifest.evm_routing_bindings.clear();
    assert!(validate_snapshot_selection(&missing).is_err());

    let mut duplicate = selection_input();
    duplicate
        .routing_manifest
        .evm_routing_bindings
        .push(EvmRoutingBinding::new("ethereum-mainnet", generation(b"other")).expect("binding"));
    assert!(validate_snapshot_selection(&duplicate).is_err());

    let mut unrelated = selection_input();
    unrelated.routing_manifest.evm_routing_bindings[0] =
        EvmRoutingBinding::new("unused-network", generation(b"unused")).expect("binding");
    assert!(validate_snapshot_selection(&unrelated).is_err());
}

#[test]
fn validator_emits_exact_ordered_collection_positions() {
    let selection = validate_snapshot_selection(&selection_input()).expect("valid selection");
    assert_eq!(
        selection.version(),
        VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION
    );
    assert_eq!(selection.target().as_str(), "portfolio-main");
    assert_eq!(selection.portfolio(), &evm_portfolio());
    assert_eq!(selection.positions().len(), 1);
    let position = &selection.positions()[0];
    assert_eq!(position.position_ordinal(), 0);
    assert_eq!(position.binding().network_id(), "ethereum-mainnet");
    assert_eq!(position.native_decimals(), 18);
    assert_eq!(position.sources().len(), 2);
    assert_eq!(position.token_contracts(), &[TOKEN.to_owned()]);
    assert_eq!(
        position
            .collection_config()
            .expect("collection projection")
            .sources(),
        position.sources()
    );
}

#[test]
fn validator_rejects_target_route_and_order_disagreement() {
    let mut wrong_target = selection_input();
    wrong_target.selector =
        PortfolioSnapshotSelector::new(crate::PortfolioId::new("portfolio-other").expect("target"));
    assert!(validate_snapshot_selection(&wrong_target).is_err());

    let mut wrong_generation = selection_input();
    wrong_generation.routing_manifest.evm_routing_bindings[0].routing_generation_ref =
        generation(b"other-generation");
    let selection =
        validate_snapshot_selection(&wrong_generation).expect("generation remains exact input");
    assert_eq!(
        selection.positions()[0].binding().routing_generation_ref(),
        &generation(b"other-generation")
    );

    let extra = EvmRoutingBinding::new("unused-network", generation(b"unused")).expect("binding");
    let mut extra_route = selection_input();
    extra_route
        .routing_manifest
        .evm_routing_bindings
        .push(extra);
    assert!(validate_snapshot_selection(&extra_route).is_err());

    let mut invalid_version = selection_input();
    invalid_version.routing_manifest.version = "mfm.portfolio.routing-manifest.v2".to_owned();
    assert!(validate_snapshot_selection(&invalid_version).is_err());
}

#[test]
fn bitcoin_demand_is_rejected_before_authoring() {
    let portfolio = decode_portfolio_config(&json!({
        "portfolio_id": "portfolio-bitcoin",
        "quote_codes": ["USD"],
        "networks": [{
            "family": "bitcoin",
            "network_id": "bitcoin-mainnet",
            "bitcoin_network": "main",
            "source_identity": "bitcoin-core-main",
            "metadata": {}
        }],
        "wallets": [{
            "wallet_id": "wallet-bitcoin",
            "subject": {
                "kind": "bitcoin_address",
                "address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"
            },
            "network_id": "bitcoin-mainnet",
            "implementation": {"kind": "address_only"},
            "symbol_ids": ["btc-native"],
            "metadata": {}
        }],
        "symbol_configs": [{
            "symbol_id": "btc-native",
            "display_symbol": "BTC",
            "network_id": "bitcoin-mainnet",
            "source": {"kind": "native"},
            "valuation": {"quotes": [{
                "quote": "USD",
                "priced_symbol_id": "btc-native",
                "unit_price_dec": "60000"
            }]},
            "metadata": {}
        }],
        "metadata": {}
    }))
    .expect("model-valid Bitcoin portfolio");
    let input = PortfolioSnapshotSelectionInput::new(
        PortfolioSnapshotSelector::new(
            crate::PortfolioId::new("portfolio-bitcoin").expect("target"),
        ),
        portfolio,
        PortfolioRoutingManifest::new(Vec::new()).expect("empty manifest"),
    );
    assert!(validate_snapshot_selection(&input).is_err());
}

#[test]
fn validated_selection_and_same_run_collections_build_snapshot_and_report() {
    let selection = selection();
    let collection = collection(&selection);
    let snapshot =
        assemble_snapshot(&selection, std::slice::from_ref(&collection)).expect("snapshot");
    assert_eq!(snapshot.network_pins.len(), 1);
    assert_eq!(snapshot.wallets.len(), 1);
    assert_eq!(snapshot.wallets[0].observations.len(), 2);
    let eth = snapshot.wallets[0]
        .observations
        .iter()
        .find(|observation| observation.symbol_id == "eth-native")
        .expect("ETH observation");
    assert_eq!(eth.quantity.amount_dec, "2.000000000000000000");
    assert_eq!(eth.values[0].value_dec, "4000.000000000000000000");

    let report = project_report(&snapshot).expect("report");
    assert_eq!(report.wallet_summaries.len(), 1);
    assert_eq!(report.totals_by_quote[0].total_value_dec, "4003.5");

    let mut substituted: serde_json::Value =
        serde_json::to_value(collection).expect("collection JSON");
    substituted["source"]["source"]["binding"]["chain_id"] = json!(10);
    let substituted = serde_json::from_value(substituted).expect("typed substituted collection");
    assert!(assemble_snapshot(&selection, &[substituted]).is_err());
}

#[test]
fn portfolio_states_are_pure_and_have_distinct_contracts() {
    let validate =
        ValidatePortfolioSnapshotSelectionState::state_contract_ref().expect("validate contract");
    let assemble = AssemblePortfolioSnapshotState::state_contract_ref().expect("assemble contract");
    let report = ProjectPortfolioReportState::state_contract_ref().expect("report contract");
    assert_ne!(validate, assemble);
    assert_ne!(assemble, report);
    assert!(matches!(validation_execution(), StateExecution::Pure(_)));
    assert!(matches!(
        snapshot_assembly_execution(),
        StateExecution::Pure(_)
    ));
    assert!(matches!(
        report_projection_execution(),
        StateExecution::Pure(_)
    ));
}

#[test]
fn compiled_sources_exactly_match_native_and_token_demand() {
    let config = selection().positions()[0]
        .collection_config()
        .expect("compiled collection");
    assert_eq!(config.sources().len(), 2);
    assert!(config.sources().iter().any(|source| {
        matches!(source.asset(), EvmBalanceAsset::Native) && source.account() == ACCOUNT
    }));
    assert!(config
        .sources()
        .iter()
        .any(|source| { source.asset().contract_address() == Some(TOKEN) }));
}
