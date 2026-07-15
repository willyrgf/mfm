use super::*;

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

#[test]
fn multiset_plan_matching_consumes_first_unmatched_identical_plan() {
    use std::collections::BTreeSet;

    let portfolio = dual_wallet_same_network_portfolio();
    let config = SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects"),
    );
    let requirements = expand_required_holdings(&config, &subjects).expect("requirements");
    let request = holding_fact_index_request(&config, &requirements[0]).expect("request");
    let expected_requests = vec![request.clone(), request];
    let first_plan = expected_requests[0].plan().clone();

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
