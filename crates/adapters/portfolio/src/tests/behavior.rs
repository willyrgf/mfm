use super::*;

#[test]
fn portfolio_runner_registration_keeps_factory_identity_explicit() {
    assert_eq!(PURE_FACTORY, "pure");
    assert_eq!(READ_FACTORY, "read_external");
    assert_eq!(ADAPTER_FACTORY, "portfolio_adapter");
}

#[tokio::test]
async fn selects_exact_receipt_facts_and_projects_receipt_pins() {
    let portfolio = dual_mainnet_portfolio();
    let btc = btc_fixture(
        BtcAddressBalanceResponse::new(
            850_000,
            BTC_HASH,
            100_000,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("Bitcoin response"),
        1,
    );
    let evm = evm_native_fixture(
        EVM_ACCOUNT,
        EvmAddressNativeBalanceResponse::new(
            21_000_000,
            EVM_HASH,
            "1000000000000000000",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("EVM response"),
        2,
    );
    let input = select_input_for_dual_mainnet(&btc, &evm);
    let artifacts = MockArtifacts::with_fixtures(&[btc.clone(), evm.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([
        (
            format!("bitcoin.address_balance_snapshot:{BTC_ADDRESS}"),
            vec![(btc.fact_ref.clone(), 17)],
        ),
        (
            format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
            vec![(evm.fact_ref.clone(), 19)],
        ),
    ]));

    let (selected, evidences) = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(portfolio.clone()).expect("config"))
            .expect("validated config"),
        input.clone(),
        &artifacts,
        &fact_index,
    )
    .await
    .expect("receipt-pinned select");

    assert_eq!(fact_index.calls(), 1);
    assert_eq!(selected.observations.len(), 2);
    assert_eq!(evidences.len(), 2);
    assert_eq!(
        project_network_pins_from_observations(&selected.observations).expect("pins"),
        input.receipt.network_anchors()
    );
    assert!(selected
        .observations
        .iter()
        .any(|observation| observation.quantity.raw_dec == "100000"));
    assert!(selected
        .observations
        .iter()
        .any(|observation| observation.quantity.raw_dec == "1000000000000000000"));

    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(portfolio).expect("snapshot config"),
        AssembleSnapshotInput {
            holdings: selected,
            receipt: input.receipt,
        },
    )
    .expect("snapshot");
    let report = mfm_state_portfolio::project_report_from_snapshot(snapshot).expect("report");
    assert_eq!(report.portfolio_id, "dual-mainnet");
}

#[tokio::test]
async fn identity_filtering_precedes_order_for_same_anchor_conflicts() {
    let target = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 10);
    let conflicting = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "999"), 11);
    let input = select_input_for_evm_native(&target, 42, EVM_HASH);
    let artifacts = MockArtifacts::with_fixtures(&[target.clone(), conflicting.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([(
        format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
        vec![
            (conflicting.fact_ref.clone(), 99),
            (target.fact_ref.clone(), 1),
        ],
    )]));

    let (selected, evidence) = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect("target identity must survive conflicting newer content");

    assert_eq!(selected.observations[0].quantity.raw_dec, "1");
    assert_eq!(evidence[0].selection().selected_indices(), &[1]);
}

#[tokio::test]
async fn identical_content_claims_are_ordered_only_after_identity_filtering() {
    let first = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 20);
    let second = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 21);
    let input = select_input_for_evm_native(&first, 42, EVM_HASH);
    let artifacts = MockArtifacts::with_fixtures(&[first.clone(), second.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([(
        format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
        vec![(first.fact_ref.clone(), 7), (second.fact_ref.clone(), 7)],
    )]));

    let (_, evidence) = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect("identical content claims are valid candidates");

    assert_eq!(
        evidence[0].selection().selected_indices(),
        &[1],
        "higher claim coordinate wins only after both rows match the receipt identity"
    );
}

#[tokio::test]
async fn newer_fact_at_another_anchor_cannot_replace_the_receipt_anchor() {
    let target = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 30);
    let newer = evm_native_fixture(
        EVM_ACCOUNT,
        native_response(
            43,
            "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "2",
        ),
        31,
    );
    let input = select_input_for_evm_native(&target, 42, EVM_HASH);
    let artifacts = MockArtifacts::with_fixtures(&[target, newer.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([(
        format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
        vec![(newer.fact_ref.clone(), 100)],
    )]));

    let error = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect_err("another anchor must not be accepted");
    assert!(error.to_string().contains("receipt_mismatch"));
}

#[tokio::test]
async fn saturation_fails_before_any_candidate_hydration() {
    let target = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 40);
    let input = select_input_for_evm_native(&target, 42, EVM_HASH);
    let artifacts = MockArtifacts::with_fixtures(std::slice::from_ref(&target));
    let fact_index = MockFactIndex::with_saturated_rows(HashMap::from([(
        format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
        std::iter::repeat_n((target.fact_ref.clone(), 1), 11).collect(),
    )]));

    let error = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect_err("N + 1 candidates must fail");
    assert!(error.to_string().contains("candidate_bound_exhausted"));
    assert_eq!(
        artifacts.reads(),
        0,
        "saturation is checked before hydration"
    );
}

#[tokio::test]
async fn any_saturated_receipt_query_blocks_hydration_for_every_holding() {
    let btc = btc_fixture(
        BtcAddressBalanceResponse::new(
            850_000,
            BTC_HASH,
            100_000,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("Bitcoin response"),
        45,
    );
    let evm = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 46);
    let input = select_input_for_dual_mainnet(&btc, &evm);
    let artifacts = MockArtifacts::with_fixtures(&[btc.clone(), evm.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([
        (
            format!("bitcoin.address_balance_snapshot:{BTC_ADDRESS}"),
            vec![(btc.fact_ref.clone(), 1)],
        ),
        (
            format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
            std::iter::repeat_n((evm.fact_ref.clone(), 1), 11).collect(),
        ),
    ]));

    let error = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(dual_mainnet_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect_err("a saturated sibling query must fail the entire receipt selection");
    assert!(error.to_string().contains("candidate_bound_exhausted"));
    assert_eq!(
        artifacts.reads(),
        0,
        "all N + 1 proofs complete before any holding candidate is hydrated"
    );
}

#[tokio::test]
async fn zero_native_and_token_facts_remain_zero_observations() {
    let native = evm_native_fixture(EVM_ACCOUNT, native_response(25, EVM_HASH, "0"), 50);
    let token = evm_erc20_fixture(
        EVM_ACCOUNT,
        TOKEN,
        EvmAddressErc20BalanceResponse::new(25, EVM_HASH, "0", 6).expect("token response"),
        51,
    );
    let input = select_input_for_evm_native_and_erc20(&native, &token, 25, EVM_HASH);
    let artifacts = MockArtifacts::with_fixtures(&[native.clone(), token.clone()]);
    let fact_index = MockFactIndex::with_rows(HashMap::from([
        (
            format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
            vec![(native.fact_ref.clone(), 1)],
        ),
        (
            format!("evm.address_erc20_balance_snapshot:{EVM_ACCOUNT}"),
            vec![(token.fact_ref.clone(), 2)],
        ),
    ]));

    let (selected, _) = select_holdings(
        ValidatedConfig::new(
            SelectHoldingsConfig::new(evm_native_and_erc20_portfolio()).expect("config"),
        )
        .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect("zero balances are successful holdings");
    assert_eq!(selected.observations.len(), 2);
    assert!(selected
        .observations
        .iter()
        .all(|observation| observation.quantity.raw_dec == "0"));
}

#[tokio::test]
async fn missing_identity_and_tampered_reference_components_hard_fail() {
    let target = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 60);
    let different_content = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "2"), 61);
    let input = select_input_for_evm_native(&target, 42, EVM_HASH);
    let no_match_artifacts = MockArtifacts::with_fixtures(std::slice::from_ref(&different_content));
    let no_match_index = MockFactIndex::with_rows(HashMap::from([(
        format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
        vec![(different_content.fact_ref.clone(), 1)],
    )]));
    let error = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"))
            .expect("validated config"),
        input.clone(),
        &no_match_artifacts,
        &no_match_index,
    )
    .await
    .expect_err("same-anchor response content must match the receipt");
    assert!(error.to_string().contains("missing_fact"));

    for (descriptor_hash, subject_hash, label) in [
        (
            digest(0x61),
            target.identity.subject_material_hash().clone(),
            "descriptor",
        ),
        (
            target.identity.fact_descriptor_hash().clone(),
            digest(0x62),
            "subject",
        ),
    ] {
        let forged = forged_reference(
            &target,
            descriptor_hash,
            subject_hash,
            target.identity.response_schema_id().clone(),
            target.identity.response_hash().clone(),
            70,
        );
        let artifacts = MockArtifacts::with_fixtures(std::slice::from_ref(&target));
        let fact_index = MockFactIndex::with_rows(HashMap::from([(
            format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
            vec![(forged, 1)],
        )]));
        let error = select_holdings(
            ValidatedConfig::new(
                SelectHoldingsConfig::new(evm_native_portfolio()).expect("config"),
            )
            .expect("validated config"),
            input.clone(),
            &artifacts,
            &fact_index,
        )
        .await
        .expect_err(&format!("{label} tampering must fail"));
        assert!(error.to_string().contains("receipt_mismatch"));
    }
}

#[tokio::test]
async fn mixed_fact_index_frontiers_hard_fail() {
    let btc = btc_fixture(
        BtcAddressBalanceResponse::new(
            850_000,
            BTC_HASH,
            1,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("Bitcoin response"),
        80,
    );
    let evm = evm_native_fixture(EVM_ACCOUNT, native_response(21_000_000, EVM_HASH, "1"), 81);
    let input = select_input_for_dual_mainnet(&btc, &evm);
    let artifacts = MockArtifacts::with_fixtures(&[btc.clone(), evm.clone()]);
    let fact_index = MockFactIndex::with_mixed_frontiers(HashMap::from([
        (
            format!("bitcoin.address_balance_snapshot:{BTC_ADDRESS}"),
            vec![(btc.fact_ref, 1)],
        ),
        (
            format!("evm.address_native_balance_snapshot:{EVM_ACCOUNT}"),
            vec![(evm.fact_ref, 1)],
        ),
    ]));

    let error = select_holdings(
        ValidatedConfig::new(SelectHoldingsConfig::new(dual_mainnet_portfolio()).expect("config"))
            .expect("validated config"),
        input,
        &artifacts,
        &fact_index,
    )
    .await
    .expect_err("mixed snapshot frontiers");
    assert!(error.to_string().contains("mixed read frontiers"));
}

#[test]
fn receipt_constrained_requests_bind_anchor_status_scope_and_limit() {
    let target = evm_native_fixture(EVM_ACCOUNT, native_response(42, EVM_HASH, "1"), 90);
    let input = select_input_for_evm_native(&target, 42, EVM_HASH);
    let config = SelectHoldingsConfig::new(evm_native_portfolio()).expect("config");
    let request =
        holding_fact_index_request(&config, &input.receipt.holdings()[0]).expect("request");
    let plan = request.plan();
    assert_eq!(plan.limit(), Some(11));
    assert_eq!(plan.store_scope().as_str(), "mfm.store.default");
    let query = plan.canonical_query().as_str();
    for field in [
        "subject.account",
        "result.block_number",
        "result.block_hash",
        "result.coverage",
        "result.source_status",
    ] {
        assert!(query.contains(field), "receipt request omitted {field}");
    }
}

fn native_response(
    block_number: u64,
    block_hash: &str,
    raw_wei: &str,
) -> EvmAddressNativeBalanceResponse {
    EvmAddressNativeBalanceResponse::new(
        block_number,
        block_hash,
        raw_wei,
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("native response")
}
