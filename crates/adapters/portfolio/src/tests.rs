//! Adapter tests for fact-backed SelectHoldings (Platform fact-index + hydrate).
//!
//! These exercise the shipped `select_holdings` path with retained FactResponse
//! artifacts — not hand-built Observations.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

use mfm_canonical::sha256_digest_bytes;
use mfm_facts::{
    fact_descriptor_hash, DescriptorCatalogWatermark, FactAudience, FactClaimId,
    FactFieldValueType, FactProducerProvenance, FactQueryReceipt, FactQueryResultRow,
    FactQueryScope, FactResponseEvidence, FactSubjectRef, FactVisibility, FactVisibilityScope,
    InternalFactRef, InternalFactRefParts, StoreCommitOrder, StoreReadFrontier, StoreScopeRef,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig,
};
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
    SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use mfm_program::MfmFactType;
use mfm_program::ValidatedConfig;
use mfm_state_portfolio::{
    project_network_pins_from_observations, resolve_subjects_from_config, ResolveSubjectsConfig,
};
use mfm_states_btc::{BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact};
use mfm_states_evm::{EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact};
use mfm_store::v1::test_support::{
    fact_query_receipt_for_test, FactQueryReceiptFixtureInputForTest,
};

#[test]
fn portfolio_runner_registration_keeps_factory_identity_explicit() {
    assert_eq!(PURE_FACTORY, "pure");
    assert_eq!(READ_FACTORY, "read_external");
    assert_eq!(ADAPTER_FACTORY, "portfolio_adapter");
}

/// Dual-mainnet native facts admitted as retained FactResponse artifacts; Platform
/// fact-index returns them; SelectHoldings hydrates and selects with providers unbound.
#[tokio::test]
async fn select_holdings_succeeds_from_platform_facts_with_providers_unbound() {
    let portfolio = dual_mainnet_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );

    let btc_response = BtcAddressBalanceResponse::new(
        850_000,
        "ab".repeat(32),
        100_000,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("btc response");

    let evm_response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        "0x".to_owned() + &"cd".repeat(32),
        "1000000000000000000",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("evm response");

    let (btc_bytes, btc_evidence, btc_ref) =
        holding_artifact_and_ref(&btc_response, "bitcoin.address_balance_snapshot", 1, 17);
    let (evm_bytes, evm_evidence, evm_ref) =
        holding_artifact_and_ref(&evm_response, "evm.address_native_balance_snapshot", 2, 19);

    let artifacts = MockArtifacts::with_map(HashMap::from([
        (btc_ref.artifact_id().clone(), (btc_bytes, btc_evidence)),
        (evm_ref.artifact_id().clone(), (evm_bytes, evm_evidence)),
    ]));
    let fact_index = MockFactIndex::with_plan_refs(HashMap::from([
        (
            "bitcoin.address_balance_snapshot".to_owned(),
            vec![(btc_ref, 17)],
        ),
        (
            "evm.address_native_balance_snapshot".to_owned(),
            vec![(evm_ref, 19)],
        ),
    ]));

    let validated = ValidatedConfig::new(config).expect("validated");
    let (selected, evidences) = select_holdings(validated, subjects, &artifacts, &fact_index)
        .await
        .expect("select holdings from Platform facts");

    // No live chain providers; one shared-frontier batch fact-index read + retained artifacts.
    assert_eq!(fact_index.calls(), 1);
    assert_eq!(selected.observations.len(), 2);
    assert_eq!(evidences.len(), 2);

    let pins = project_network_pins_from_observations(&selected.observations).expect("pins");
    assert_eq!(pins.len(), 2);

    let btc_obs = selected
        .observations
        .iter()
        .find(|o| o.network_id == "bitcoin-mainnet")
        .expect("btc observation");
    let evm_obs = selected
        .observations
        .iter()
        .find(|o| o.network_id == "ethereum-mainnet")
        .expect("evm observation");

    assert_eq!(btc_obs.quantity.raw_dec, "100000");
    assert_eq!(btc_obs.coverage, "configured_only");
    assert_eq!(
        btc_obs.source.anchor,
        ObservationAnchor::Bitcoin {
            height: 850_000,
            block_hash: "ab".repeat(32),
        }
    );
    assert_eq!(evm_obs.quantity.raw_dec, "1000000000000000000");
    assert_eq!(evm_obs.coverage, "configured_only");
    assert_eq!(
        evm_obs.source.anchor,
        ObservationAnchor::Evm {
            chain_id: 1,
            block_number: 21_000_000,
            block_hash: "0x".to_owned() + &"cd".repeat(32),
        }
    );

    let btc_pin = pins
        .iter()
        .find(|p| p.network_id == "bitcoin-mainnet")
        .expect("btc pin");
    let evm_pin = pins
        .iter()
        .find(|p| p.network_id == "ethereum-mainnet")
        .expect("evm pin");
    assert_eq!(
        btc_pin.anchor,
        ExecutionAnchor::Bitcoin {
            height: 850_000,
            block_hash: "ab".repeat(32),
        }
    );
    assert_eq!(
        evm_pin.anchor,
        ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 21_000_000,
            block_hash: "0x".to_owned() + &"cd".repeat(32),
        }
    );

    // Continue shipped pure report path: valuations + assemble + project (no live chain).
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.symbol_configs.clone()).expect("valuations config"),
    )
    .expect("resolve valuations");
    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(2, portfolio.clone()).expect("assemble config"),
        AssembleSnapshotInput {
            subjects: resolve_subjects_from_config(
                &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects"),
            ),
            holdings: selected,
            valuations,
        },
        0,
    )
    .expect("assemble snapshot from selected holdings");
    assert_eq!(snapshot.network_pins, pins);
    assert_eq!(snapshot.wallets.len(), 2);
    // Public snapshot JSON must retain selected coverage honesty (W1).
    let snapshot_json = serde_json::to_value(&snapshot).expect("snapshot json");
    let coverage_tags = snapshot_json
        .pointer("/wallets")
        .and_then(|w| w.as_array())
        .into_iter()
        .flatten()
        .filter_map(|wallet| wallet.get("observations")?.as_array())
        .flatten()
        .filter_map(|obs| obs.get("coverage")?.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        coverage_tags.len(),
        2,
        "coverage present on each observation"
    );
    assert!(coverage_tags.iter().all(|c| c == "configured_only"));

    let report =
        mfm_state_portfolio::project_report_from_snapshot(snapshot, 2).expect("project report");
    assert_eq!(report.portfolio_id, "dual-mainnet");
    assert_eq!(report.network_pins, pins);
}

/// Network-coherent: two holdings on one network; A@100+A@99 and B@99 → both @99.
#[tokio::test]
async fn select_holdings_network_coherent_picks_common_anchor_not_independent_latest() {
    let portfolio = dual_wallet_same_network_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );

    // Wallet A candidates @100 and @99; wallet B only @99.
    let a99 = EvmAddressNativeBalanceResponse::new(
        99,
        "0x".to_owned() + &"99".repeat(32),
        "1",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("a99");
    let a100 = EvmAddressNativeBalanceResponse::new(
        100,
        "0x".to_owned() + &"aa".repeat(32),
        "2",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("a100");
    let b99 = EvmAddressNativeBalanceResponse::new(
        99,
        "0x".to_owned() + &"99".repeat(32),
        "3",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("b99");

    let (a99_bytes, a99_ev, a99_ref) =
        holding_artifact_and_ref(&a99, "evm.address_native_balance_snapshot", 10, 1);
    let (a100_bytes, a100_ev, a100_ref) =
        holding_artifact_and_ref(&a100, "evm.address_native_balance_snapshot", 11, 2);
    let (b99_bytes, b99_ev, b99_ref) =
        holding_artifact_and_ref(&b99, "evm.address_native_balance_snapshot", 12, 3);

    let artifacts = MockArtifacts::with_map(HashMap::from([
        (a99_ref.artifact_id().clone(), (a99_bytes, a99_ev)),
        (a100_ref.artifact_id().clone(), (a100_bytes, a100_ev)),
        (b99_ref.artifact_id().clone(), (b99_bytes, b99_ev)),
    ]));

    // Map by wallet address predicate — mock returns candidates based on request plan kind.
    // Both holdings share the same fact kind; dispatch by subject.account in the mock.
    let fact_index = MockFactIndex::with_account_refs(HashMap::from([
        (
            "0x000000000000000000000000000000000000000a".to_owned(),
            vec![(a100_ref, 10), (a99_ref, 5)],
        ),
        (
            "0x000000000000000000000000000000000000000b".to_owned(),
            vec![(b99_ref, 7)],
        ),
    ]));

    let validated = ValidatedConfig::new(config).expect("validated");
    let (selected, _) = select_holdings(validated, subjects, &artifacts, &fact_index)
        .await
        .expect("network-coherent select");

    assert_eq!(selected.observations.len(), 2);
    for obs in &selected.observations {
        match &obs.source.anchor {
            ObservationAnchor::Evm {
                block_number,
                block_hash,
                ..
            } => {
                assert_eq!(*block_number, 99, "must select common @99 not A@100");
                assert_eq!(block_hash, &("0x".to_owned() + &"99".repeat(32)));
            }
            other => panic!("expected EVM anchor, got {other:?}"),
        }
    }
    let pins = project_network_pins_from_observations(&selected.observations).expect("pins");
    assert_eq!(pins.len(), 1);
    assert_eq!(
        pins[0].anchor,
        ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 99,
            block_hash: "0x".to_owned() + &"99".repeat(32),
        }
    );
}

#[tokio::test]
async fn select_holdings_hard_fails_when_platform_facts_missing() {
    let portfolio = dual_mainnet_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );
    let artifacts = MockArtifacts::with_map(HashMap::new());
    let fact_index = MockFactIndex::with_plan_refs(HashMap::new());
    let validated = ValidatedConfig::new(config).expect("validated");
    let err = select_holdings(validated, subjects, &artifacts, &fact_index)
        .await
        .expect_err("missing facts");
    let msg = err.to_string();
    assert!(
        msg.contains("missing_fact") || msg.contains("no acceptable"),
        "expected missing_fact hard-fail, got {msg}"
    );
}

#[tokio::test]
async fn select_holdings_hard_fails_on_mixed_read_frontiers() {
    let portfolio = dual_mainnet_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );

    let btc_response = BtcAddressBalanceResponse::new(
        850_000,
        "ab".repeat(32),
        100_000,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("btc response");
    let evm_response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        "0x".to_owned() + &"cd".repeat(32),
        "1000000000000000000",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("evm response");
    let (btc_bytes, btc_evidence, btc_ref) =
        holding_artifact_and_ref(&btc_response, "bitcoin.address_balance_snapshot", 1, 17);
    let (evm_bytes, evm_evidence, evm_ref) =
        holding_artifact_and_ref(&evm_response, "evm.address_native_balance_snapshot", 2, 19);
    let artifacts = MockArtifacts::with_map(HashMap::from([
        (btc_ref.artifact_id().clone(), (btc_bytes, btc_evidence)),
        (evm_ref.artifact_id().clone(), (evm_bytes, evm_evidence)),
    ]));
    let fact_index = MixedFrontierFactIndex {
        by_kind: HashMap::from([
            (
                "bitcoin.address_balance_snapshot".to_owned(),
                vec![(btc_ref, 17)],
            ),
            (
                "evm.address_native_balance_snapshot".to_owned(),
                vec![(evm_ref, 19)],
            ),
        ]),
    };
    let validated = ValidatedConfig::new(config).expect("validated");
    let err = select_holdings(validated, subjects, &artifacts, &fact_index)
        .await
        .expect_err("mixed frontiers must hard-fail");
    let msg = err.to_string();
    assert!(
        msg.contains("runner_output_invalid") && msg.contains("mixed read frontiers"),
        "expected runner_output_invalid hard-fail, got {msg}"
    );
}

/// Candidates present but all fail coverage/status filter → hard missing_fact (not soft success).
#[tokio::test]
async fn select_holdings_hard_fails_when_only_truncated_candidates_present() {
    let portfolio = dual_mainnet_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );

    // Truncated is not constructible via Response::new (write-admission fail-closed).
    // Build inadmissible rows the same way a tampered/legacy payload would hydrate.
    let truncated_btc = serde_json::from_value::<BtcAddressBalanceResponse>(serde_json::json!({
        "anchor_height": 850_000,
        "anchor_hash": "ab".repeat(32),
        "balance_sats": 100_000,
        "coverage": "truncated",
        "source_status": "ok"
    }))
    .expect("truncated btc response");
    let truncated_evm =
        serde_json::from_value::<EvmAddressNativeBalanceResponse>(serde_json::json!({
            "block_number": 21_000_000,
            "block_hash": "0x".to_owned() + &"cd".repeat(32),
            "raw_wei": "1000000000000000000",
            "decimals": 18,
            "coverage": "truncated",
            "source_status": "ok"
        }))
        .expect("truncated evm response");

    let (btc_bytes, btc_evidence, btc_ref) =
        holding_artifact_and_ref(&truncated_btc, "bitcoin.address_balance_snapshot", 3, 17);
    let (evm_bytes, evm_evidence, evm_ref) =
        holding_artifact_and_ref(&truncated_evm, "evm.address_native_balance_snapshot", 4, 19);

    let artifacts = MockArtifacts::with_map(HashMap::from([
        (btc_ref.artifact_id().clone(), (btc_bytes, btc_evidence)),
        (evm_ref.artifact_id().clone(), (evm_bytes, evm_evidence)),
    ]));
    let fact_index = MockFactIndex::with_plan_refs(HashMap::from([
        (
            "bitcoin.address_balance_snapshot".to_owned(),
            vec![(btc_ref, 17)],
        ),
        (
            "evm.address_native_balance_snapshot".to_owned(),
            vec![(evm_ref, 19)],
        ),
    ]));

    let validated = ValidatedConfig::new(config).expect("validated");
    let err = select_holdings(validated, subjects, &artifacts, &fact_index)
        .await
        .expect_err("truncated-only candidates must hard-fail");
    let msg = err.to_string();
    assert!(
        msg.contains("missing_fact") || msg.contains("no acceptable"),
        "coverage filter-empty must surface missing_fact, got {msg}"
    );
}

// --- fixtures ---

fn dual_mainnet_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "dual-mainnet".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![
            NetworkConfig::new(
                "ethereum-mainnet".to_owned(),
                NetworkFamilyConfig::Evm,
                Some(1),
                None,
                None,
                BTreeMap::new(),
            )
            .expect("evm network"),
            NetworkConfig::new(
                "bitcoin-mainnet".to_owned(),
                NetworkFamilyConfig::Bitcoin,
                None,
                Some("main".to_owned()),
                Some("public-bitcoin-core".to_owned()),
                BTreeMap::new(),
            )
            .expect("btc network"),
        ],
        wallets: vec![
            WalletConfig {
                wallet_id: "wallet_eth".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000dead",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
            WalletConfig {
                wallet_id: "wallet_btc".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
                    WalletSubjectKind::BitcoinAddress,
                )
                .expect("subject"),
                network_id: "bitcoin-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["btc.native.bitcoin-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
        ],
        symbol_configs: vec![
            SymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet".parse().expect("symbol"),
                display_symbol: Some("ETH".to_owned()),
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Native,
                network_id: "ethereum-mainnet".parse().expect("network"),
                protocol: None,
                balance_reader: BalanceReaderConfig::NativeBalance {},
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "eth.native.ethereum-mainnet".parse().expect("priced"),
                        unit_price_dec: "2.5".parse().expect("price"),
                    }],
                },
                underlying_symbol_id: None,
                metadata: PublicMetadata::default(),
            },
            SymbolConfig {
                symbol_id: "btc.native.bitcoin-mainnet".parse().expect("symbol"),
                display_symbol: Some("BTC".to_owned()),
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Native,
                network_id: "bitcoin-mainnet".parse().expect("network"),
                protocol: None,
                balance_reader: BalanceReaderConfig::NativeBalance {},
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "btc.native.bitcoin-mainnet".parse().expect("priced"),
                        unit_price_dec: "1".parse().expect("price"),
                    }],
                },
                underlying_symbol_id: None,
                metadata: PublicMetadata::default(),
            },
        ],
        metadata: PublicMetadata::default(),
    }
}

fn dual_wallet_same_network_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "coherent".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            None,
            None,
            BTreeMap::new(),
        )
        .expect("evm network")],
        wallets: vec![
            WalletConfig {
                wallet_id: "wallet_a".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000000a",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
            WalletConfig {
                wallet_id: "wallet_b".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000000b",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
        ],
        symbol_configs: vec![SymbolConfig {
            symbol_id: "eth.native.ethereum-mainnet".parse().expect("symbol"),
            display_symbol: Some("ETH".to_owned()),
            kind: SymbolKind::NativeBalance,
            role: SymbolRole::Native,
            network_id: "ethereum-mainnet".parse().expect("network"),
            protocol: None,
            balance_reader: BalanceReaderConfig::NativeBalance {},
            valuation: SymbolValuationConfig {
                quotes: vec![QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet".parse().expect("priced"),
                    unit_price_dec: "1".parse().expect("price"),
                }],
            },
            underlying_symbol_id: None,
            metadata: PublicMetadata::default(),
        }],
        metadata: PublicMetadata::default(),
    }
}

fn holding_artifact_and_ref<T: serde::Serialize>(
    response: &T,
    fact_kind: &str,
    seed: u8,
    store_commit_order: u64,
) -> (Vec<u8>, store::ArtifactEvidenceRef, InternalFactRef) {
    let _ = store_commit_order; // carried on receipt row, not artifact
    let bytes = serde_json::to_vec(response).expect("response json");
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let producer = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 20));
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: bytes.len() as u64,
        media_type: mfm_spec::v1::MediaType::new("application/json").expect("media"),
        schema_id: Some(schema_id(seed + 30)),
        semantic_type_id: None,
        producer_node_id: Some(producer.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    let artifact_evidence_hash = evidence.evidence_hash().expect("evidence hash");
    let descriptor_hash = match fact_kind {
        "bitcoin.address_balance_snapshot" => {
            fact_descriptor_hash(&BtcAddressBalanceSnapshotFact::descriptor().expect("d"))
                .expect("hash")
        }
        "evm.address_native_balance_snapshot" => {
            fact_descriptor_hash(&EvmAddressNativeBalanceSnapshotFact::descriptor().expect("d"))
                .expect("hash")
        }
        other => panic!("unknown fact kind {other}"),
    };
    let fact_ref = InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(seed), u64::from(seed), 0).expect("claim"),
        source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 1)),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        producer_node_id: producer,
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new(fact_kind).expect("kind"),
        fact_descriptor_hash: descriptor_hash,
        subject: FactSubjectRef::new(
            digest_seed(seed + 3),
            mfm_facts::FactKey::from_digest(digest_seed(seed + 4)),
            digest_seed(seed + 5),
        ),
        request: None,
        response: FactResponseEvidence::new(
            schema_id(seed + 30),
            digest,
            artifact_id,
            artifact_evidence_hash,
        ),
        producer: FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "fact.record",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 10),
            )
            .expect("cap"),
            CapabilityVersion::new("mfm.test.fact.record.v1").expect("ver"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 11),
            )
            .expect("adapter"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter ver"),
        ),
    })
    .expect("fact ref");
    (bytes, evidence, fact_ref)
}

struct MockFactIndex {
    calls: Mutex<usize>,
    by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>,
    by_account: HashMap<String, Vec<(InternalFactRef, u64)>>,
}

impl MockFactIndex {
    fn with_plan_refs(by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>) -> Self {
        Self {
            calls: Mutex::new(0),
            by_kind,
            by_account: HashMap::new(),
        }
    }

    fn with_account_refs(by_account: HashMap<String, Vec<(InternalFactRef, u64)>>) -> Self {
        Self {
            calls: Mutex::new(0),
            by_kind: HashMap::new(),
            by_account,
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("calls")
    }
}

impl FactIndexReadProvider for MockFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            // One batch call counts as one shared-frontier selection read.
            *self.calls.lock().expect("calls") += 1;
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                let plan = request.plan();
                let kind = plan_fact_kind(plan);
                let account = plan_account_predicate(plan);

                // Prefer account-keyed fixtures (same-network multi-wallet tests); fall
                // back to fact-kind keyed fixtures (dual-mainnet BTC+EVM).
                let refs_with_order: Vec<(InternalFactRef, u64)> = account
                    .as_ref()
                    .and_then(|acct| self.by_account.get(acct.as_str()).cloned())
                    .or_else(|| self.by_kind.get(&kind).cloned())
                    .unwrap_or_default();

                let rows: Vec<FactQueryResultRow> = refs_with_order
                    .into_iter()
                    .map(|(fact_ref, order)| {
                        FactQueryResultRow::new(
                            fact_ref,
                            vec![mfm_facts::FactFieldValue::new(
                                mfm_facts::FactFieldId::new("metadata.store_commit_order")
                                    .expect("field"),
                                FactFieldValueType::UnsignedInteger,
                                FactCanonicalScalar::UnsignedInteger(order),
                            )
                            .expect("field value")],
                        )
                    })
                    .collect();

                let receipt = signed_receipt_for_plan(plan, &rows);
                responses.push(
                    mfm_facts::FactQueryResult::new(
                        mfm_facts::fact_query_result_rows_from_receipt(&receipt),
                        receipt,
                    )
                    .expect("fact query result"),
                );
            }
            Ok(responses)
        })
    }
}

/// Test provider that stamps a different store-commit watermark per response.
struct MixedFrontierFactIndex {
    by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>,
}

impl FactIndexReadProvider for MixedFrontierFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.mixed-frontier.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            let mut responses = Vec::with_capacity(requests.len());
            for (index, request) in requests.iter().enumerate() {
                let plan = request.plan();
                let kind = plan_fact_kind(plan);
                let refs_with_order = self.by_kind.get(&kind).cloned().unwrap_or_default();
                let rows: Vec<FactQueryResultRow> = refs_with_order
                    .into_iter()
                    .map(|(fact_ref, order)| {
                        FactQueryResultRow::new(
                            fact_ref,
                            vec![mfm_facts::FactFieldValue::new(
                                mfm_facts::FactFieldId::new("metadata.store_commit_order")
                                    .expect("field"),
                                FactFieldValueType::UnsignedInteger,
                                FactCanonicalScalar::UnsignedInteger(order),
                            )
                            .expect("field value")],
                        )
                    })
                    .collect();
                let receipt = signed_receipt_for_plan_with_order(
                    plan,
                    &rows,
                    StoreCommitOrder::new(11 + index as u64),
                );
                responses.push(
                    mfm_facts::FactQueryResult::new(
                        mfm_facts::fact_query_result_rows_from_receipt(&receipt),
                        receipt,
                    )
                    .expect("fact query result"),
                );
            }
            Ok(responses)
        })
    }
}

fn plan_fact_kind(plan: &mfm_facts::CanonicalFactQueryPlan) -> String {
    let value: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    value["fact_kind"].as_str().expect("fact_kind").to_owned()
}

fn plan_account_predicate(plan: &mfm_facts::CanonicalFactQueryPlan) -> Option<String> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    for predicate in shape.predicates() {
        if predicate.field_id().as_str() == "subject.account" {
            if let FactCanonicalScalar::String(value) = predicate.value() {
                return Some(value.as_str().to_owned());
            }
        }
    }
    None
}

fn signed_receipt_for_plan(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    rows: &[FactQueryResultRow],
) -> FactQueryReceipt {
    signed_receipt_for_plan_with_order(plan, rows, StoreCommitOrder::new(11))
}

fn signed_receipt_for_plan_with_order(
    _plan: &mfm_facts::CanonicalFactQueryPlan,
    rows: &[FactQueryResultRow],
    store_commit_order: StoreCommitOrder,
) -> FactQueryReceipt {
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(1),
        store_commit_order,
    );
    fact_query_receipt_for_test(FactQueryReceiptFixtureInputForTest {
        read_frontier,
        rows,
        include_returned_field_summaries: true,
        limit: None,
    })
}

struct MockArtifacts {
    by_id: HashMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>,
}

impl MockArtifacts {
    fn with_map(by_id: HashMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>) -> Self {
        Self { by_id }
    }
}

impl store::RetainedArtifactReadProvider for MockArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let Some((bytes, evidence)) = self.by_id.get(&requirement.artifact_id) else {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            store::VerifiedRunArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        })
    }
}

#[test]
fn multiset_plan_matching_consumes_first_unmatched_identical_plan() {
    use std::collections::BTreeSet;

    let portfolio = dual_wallet_same_network_portfolio();
    // Force both wallets to share one address so query plans are identical.
    let mut portfolio = portfolio;
    let shared = portfolio.wallets[0].subject.address_str().to_owned();
    portfolio.wallets[1].subject =
        WalletSubject::new(shared, WalletSubjectKind::EvmAddress).expect("shared subject");
    let config = SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects"),
    );
    let requirements = expand_required_holdings(&config, &subjects).expect("requirements");
    assert!(requirements.len() >= 2, "need at least two requirements");
    let expected_requests = requirements
        .iter()
        .map(|requirement| holding_fact_index_request(&config, requirement).expect("request"))
        .collect::<Vec<_>>();
    let first_plan = expected_requests[0].plan().clone();
    let identical_pair: Vec<_> = expected_requests
        .iter()
        .enumerate()
        .filter(|(_, request)| request.plan() == &first_plan)
        .map(|(index, _)| index)
        .collect();
    assert!(
        identical_pair.len() >= 2,
        "fixture must produce at least two identical plans for same-address wallets"
    );

    let mut matched = BTreeSet::new();
    let first = first_unmatched_plan_index(&expected_requests, &matched, &first_plan)
        .expect("first identical plan slot");
    matched.insert(first);
    let second = first_unmatched_plan_index(&expected_requests, &matched, &first_plan)
        .expect("second identical plan slot");
    assert_ne!(
        first, second,
        "duplicate plans must bind distinct requirement slots"
    );
    matched.insert(second);
    assert!(
        first_unmatched_plan_index(&expected_requests, &matched, &first_plan).is_none(),
        "no third unmatched slot for the same plan"
    );
}

fn run_id(seed: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn schema_id(seed: u8) -> SchemaId {
    SchemaId::new(
        "mfm.test.fact_response",
        "v1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("schema")
}

fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}

fn digest_seed(seed: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}
