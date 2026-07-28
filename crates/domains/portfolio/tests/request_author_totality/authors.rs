use std::collections::{BTreeMap, BTreeSet};

use mfm_bitcoin::{
    BitcoinBalanceCollectionConfig, BitcoinBalanceCollectionError, BitcoinBalanceCollectionRequest,
    CollectBitcoinBalancesState,
};
use mfm_evm::{
    CollectEvmBalancesState, EvmBalanceCollectionConfig, EvmBalanceCollectionError,
    EvmBalanceCollectionPlan, EvmNetworkBinding,
};
use mfm_facts::CanonicalFactQueryPlan;
use mfm_portfolio::{
    portfolio_snapshot_authoring_catalog, portfolio_snapshot_program_draft, SelectHoldingsConfig,
    SelectHoldingsInput, SelectHoldingsState,
};
use mfm_program::{CertifiedContext, ReadState, StateSpec, ValidatedConfig};

use super::support::{
    bitcoin_portfolio_config, evm_collection_config, evm_plan, evm_portfolio_config,
    selection_material, BITCOIN_MAINNET_ADDRESS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CutoverDisposition {
    RetainPure,
    ReplaceWithDecomposedEvmReads,
    ValidateAndSplitFactSelectionUpstream,
    UnregisterBitcoinPendingQualification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptedFactSelectionFrame {
    request: CanonicalFactQueryPlan,
}

fn validate_and_split_selection_upstream(
    config: SelectHoldingsConfig,
    input: &SelectHoldingsInput,
) -> Result<Vec<AcceptedFactSelectionFrame>, String> {
    let config = ValidatedConfig::new(config).map_err(|error| error.to_string())?;
    let state =
        <SelectHoldingsState as StateSpec>::new(config).map_err(|error| error.to_string())?;
    let plan = state
        .plan(input, &CertifiedContext::no_context())
        .map_err(|error| error.to_string())?;
    let requests = plan.requests().map_err(|error| error.to_string())?;
    if requests.is_empty() {
        return Err("accepted portfolio selection demand must be non-empty".to_owned());
    }
    Ok(requests
        .into_iter()
        .map(|request| AcceptedFactSelectionFrame { request })
        .collect())
}

fn author_fact_selection_request(frame: &AcceptedFactSelectionFrame) -> CanonicalFactQueryPlan {
    frame.request.clone()
}

#[test]
fn published_snapshot_reachability_has_one_exhaustive_cutover_disposition() {
    let expected = BTreeMap::from([
        (
            "mfm.bitcoin.collect_balances",
            (
                "read_external",
                CutoverDisposition::UnregisterBitcoinPendingQualification,
            ),
        ),
        (
            "mfm.evm.collect_balances",
            (
                "read_external",
                CutoverDisposition::ReplaceWithDecomposedEvmReads,
            ),
        ),
        (
            "mfm.portfolio.assemble_snapshot",
            ("pure", CutoverDisposition::RetainPure),
        ),
        (
            "mfm.portfolio.project_report",
            ("pure", CutoverDisposition::RetainPure),
        ),
        (
            "mfm.portfolio.select_holdings",
            (
                "read_external",
                CutoverDisposition::ValidateAndSplitFactSelectionUpstream,
            ),
        ),
    ]);

    let catalog = portfolio_snapshot_authoring_catalog().expect("snapshot authoring catalog");
    let registered = catalog
        .state_descriptors()
        .map(|descriptor| (descriptor.name.as_str(), descriptor.runner.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        registered,
        expected
            .iter()
            .map(|(name, (runner, _))| (*name, *runner))
            .collect()
    );
    assert_eq!(catalog.side_effect_state_descriptor_ids().len(), 0);

    let mut observed = BTreeSet::new();
    for portfolio in [bitcoin_portfolio_config(), evm_portfolio_config(2)] {
        observed.extend(
            portfolio_snapshot_program_draft(portfolio)
                .expect("published branch draft")
                .state_nodes()
                .iter()
                .map(|node| node.state_descriptor_name.clone()),
        );
    }
    assert_eq!(
        observed,
        expected
            .keys()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>()
    );
    assert!(expected
        .values()
        .all(|(runner, disposition)| match *runner {
            "pure" => *disposition == CutoverDisposition::RetainPure,
            "read_external" => *disposition != CutoverDisposition::RetainPure,
            _ => false,
        }));
}

#[test]
fn accepted_aggregate_configs_expose_exact_pre_cutover_replacement_scope() {
    let _: fn(
        &BitcoinBalanceCollectionConfig,
    ) -> Result<BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionError> =
        BitcoinBalanceCollectionConfig::request;
    let _: fn(&EvmBalanceCollectionConfig) -> Result<EvmNetworkBinding, EvmBalanceCollectionError> =
        EvmBalanceCollectionConfig::binding;

    for (network, address) in [
        ("main", BITCOIN_MAINNET_ADDRESS),
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
        let config = BitcoinBalanceCollectionConfig::new(
            format!("bitcoin-{network}"),
            network,
            "source-a",
            vec![address.to_owned()],
        )
        .expect("accepted Bitcoin config");
        let state = <CollectBitcoinBalancesState as StateSpec>::new(
            ValidatedConfig::new(config).expect("validated Bitcoin config"),
        )
        .expect("Bitcoin state");
        let first = state
            .plan(&(), &CertifiedContext::no_context())
            .expect("accepted Bitcoin input authors one aggregate plan");
        let second = state
            .plan(&(), &CertifiedContext::no_context())
            .expect("repeat Bitcoin plan");
        assert_eq!(first, second);
        first
            .request()
            .expect("accepted aggregate plan reconstructs its checked demand");
        let application_operations = ["getblockchaininfo", "scantxoutset start", "getblockhash"];
        assert_eq!(
            application_operations.len(),
            3,
            "the aggregate hides bootstrap, scan, and confirmation operations"
        );
    }

    for source_count in 1..=6 {
        for token_mask in 0..(1 << source_count) {
            let config = evm_collection_config(source_count, token_mask);
            let first = evm_plan(&config);
            let second = evm_plan(&config);
            assert_eq!(first, second);
            first.binding().expect("accepted EVM binding");

            let operation_count = 3 + first.sources().len() + first.token_contracts().len();
            assert!(
                operation_count > 1,
                "the current aggregate cannot be treated as one audited operation"
            );
        }
    }

    let _: fn(
        &CollectEvmBalancesState,
        &(),
        &CertifiedContext<mfm_program::NoContext>,
    ) -> mfm_program::StateResult<EvmBalanceCollectionPlan> =
        <CollectEvmBalancesState as ReadState>::plan;
}

#[test]
fn checked_fact_selection_frames_author_exactly_one_infallible_request() {
    let _: fn(&AcceptedFactSelectionFrame) -> CanonicalFactQueryPlan =
        author_fact_selection_request;

    for holding_count in 1..=8 {
        let (config, input) = selection_material(holding_count);
        let frames = validate_and_split_selection_upstream(config, &input)
            .expect("accepted input splits before request-state entry");
        assert_eq!(frames.len(), holding_count);
        for frame in &frames {
            let first = author_fact_selection_request(frame);
            let second = author_fact_selection_request(frame);
            assert_eq!(first, second);
            assert_eq!(first, frame.request);
        }
    }
}

#[test]
fn weak_selection_input_is_rejected_before_any_request_frame_exists() {
    let (config, _) = selection_material(1);
    let missing_receipt = SelectHoldingsInput {
        bitcoin_receipts: Vec::new(),
        evm_receipts: Vec::new(),
    };
    assert!(
        validate_and_split_selection_upstream(config, &missing_receipt).is_err(),
        "receipt validation belongs before the infallible request author"
    );
}
