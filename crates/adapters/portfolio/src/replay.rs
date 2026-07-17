use super::*;

use mfm_portfolio_model::portfolio::PortfolioSnapshot;
use mfm_replay::v1::{self as replay, load_node_config as replay_node_config};

use super::selection::{
    holding_fact_index_request, hydrate_btc_candidate, hydrate_evm_erc20_candidate,
    hydrate_evm_native_candidate, require_shared_snapshot_read_frontier,
    store_commit_order_from_row,
};

/// Verifies the produced receipt-pinned portfolio projection prefix from retained evidence only.
///
/// The operation owner reconstructs `receipt` from family receipts before calling this adapter
/// verifier. This adapter accepts only the strict output prefixes from selection through report,
/// verifies every produced stage, and proves that each stage consumed the exact preceding value.
pub fn verify_portfolio_replay(
    broker: &replay::ReplayBroker,
    receipt: &PortfolioCollectionReceipt,
) -> replay::Result<()> {
    let select_frame =
        optional_replay_single_state_frame::<SelectHoldingsState>(broker, "SelectHoldings")?;
    let snapshot_frame =
        optional_replay_single_state_frame::<AssembleSnapshotState>(broker, "assembled snapshot")?;
    let report_frame =
        optional_replay_single_state_frame::<ProjectReportState>(broker, "projected report")?;

    if snapshot_frame.is_some() && select_frame.is_none() {
        return Err(replay_portfolio_mismatch(
            "AssembleSnapshot output was produced without a SelectHoldings predecessor",
        ));
    }
    if report_frame.is_some() && snapshot_frame.is_none() {
        return Err(replay_portfolio_mismatch(
            "ProjectReport output was produced without an AssembleSnapshot predecessor",
        ));
    }
    let Some(select_frame) = select_frame else {
        return Ok(());
    };

    let config: SelectHoldingsConfig = replay_node_config(broker, &select_frame.node)?;
    if config.selection_policy_id()
        != mfm_state_portfolio::PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID
    {
        return Err(replay_portfolio_mismatch(
            "portfolio selection policy does not match the receipt-anchor policy",
        ));
    }
    let receipt_input =
        replay_single_input_value::<PortfolioCollectionReceipt>(broker, &select_frame.node)?;
    if receipt_input != *receipt {
        return Err(replay_portfolio_mismatch(
            "SelectHoldings did not consume the exact assembled portfolio collection receipt",
        ));
    }
    validate_receipt_against_portfolio(receipt, config.portfolio())
        .map_err(replay_adapter_error)?;

    let expected_requests = receipt
        .holdings()
        .iter()
        .map(|entry| holding_fact_index_request(&config, entry).map_err(replay_adapter_error))
        .collect::<replay::Result<Vec<_>>>()?;
    let query_evidence = replay::fact_query_evidence_for_attempt(
        broker,
        &select_frame.node.node_id,
        &select_frame.produced.attempt_id,
    )?;
    if query_evidence.len() != expected_requests.len() {
        return Err(replay_portfolio_mismatch(
            "portfolio selection query evidence is incomplete or has extra reads",
        ));
    }
    require_shared_snapshot_read_frontier(
        query_evidence
            .iter()
            .map(|evidence| evidence.receipt().read_frontier()),
        query_evidence
            .iter()
            .map(|evidence| evidence.receipt().frontier_type()),
    )
    .map_err(replay_portfolio_mismatch)?;

    let mut evidence_by_entry = vec![None; receipt.holdings().len()];
    let mut matched = BTreeSet::new();
    for evidence in query_evidence {
        if evidence.selection().selection_policy_hash()
            != &mfm_state_portfolio::portfolio_holding_selection_policy_digest()
            || evidence.selection().selected_summaries_digest().is_some()
        {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence did not use the certified receipt selection policy",
            ));
        }
        let Some(index) = first_unmatched_plan_index(&expected_requests, &matched, evidence.plan())
        else {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence did not match a certified receipt-pinned request",
            ));
        };
        if evidence_by_entry[index].replace(evidence).is_some() {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence was duplicated",
            ));
        }
        matched.insert(index);
    }

    // Replay enforces the same all-queries cardinality barrier as live execution before it reads
    // any retained candidate response artifact.
    for evidence in &evidence_by_entry {
        let evidence = evidence.as_ref().ok_or_else(|| {
            replay_portfolio_mismatch("missing receipt-pinned fact query evidence")
        })?;
        require_replay_exact_bounded_cardinality(evidence, &config)?;
    }

    let mut selected = Vec::with_capacity(receipt.holdings().len());
    for (index, entry) in receipt.holdings().iter().enumerate() {
        let evidence = evidence_by_entry[index].as_ref().ok_or_else(|| {
            replay_portfolio_mismatch("missing receipt-pinned fact query evidence")
        })?;
        let (winner, selected_index) = replay_selected_candidate(broker, entry, &config, evidence)?;
        if evidence.selection().selected_indices() != [selected_index as u64] {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence selected index did not match post-identity ordering",
            ));
        }
        selected.push(winner);
    }
    selected.sort_by(|left, right| left.key.cmp(&right.key));
    let symbols =
        symbols_by_id_map(&config.portfolio().symbol_configs).map_err(replay_adapter_error)?;
    let observations =
        observations_from_selected_holdings(&selected, &symbols).map_err(replay_adapter_error)?;
    let pins =
        project_network_pins_from_observations(&observations).map_err(replay_adapter_error)?;
    if pins.as_slice() != receipt.network_anchors() {
        return Err(replay_portfolio_mismatch(
            "replayed selected anchors did not equal the collection receipt anchors",
        ));
    }
    let selected_output = SelectedHoldings { observations };
    verify_replay_output_bytes(&select_frame, &selected_output, "selected holdings")?;

    let Some(snapshot_frame) = snapshot_frame else {
        return Ok(());
    };
    let snapshot_config: AssembleSnapshotConfig = replay_node_config(broker, &snapshot_frame.node)?;
    if snapshot_config.portfolio() != config.portfolio() {
        return Err(replay_portfolio_mismatch(
            "portfolio snapshot config does not match SelectHoldings config",
        ));
    }
    let snapshot_receipt =
        replay_single_input_value::<PortfolioCollectionReceipt>(broker, &snapshot_frame.node)?;
    if snapshot_receipt != *receipt {
        return Err(replay_portfolio_mismatch(
            "AssembleSnapshot did not consume the exact assembled portfolio collection receipt",
        ));
    }
    let snapshot_holdings =
        replay_single_input_value::<SelectedHoldings>(broker, &snapshot_frame.node)?;
    if snapshot_holdings != selected_output {
        return Err(replay_portfolio_mismatch(
            "AssembleSnapshot did not consume the exact selected holdings output",
        ));
    }
    let snapshot = assemble_snapshot(
        &snapshot_config,
        AssembleSnapshotInput {
            holdings: snapshot_holdings,
            receipt: snapshot_receipt,
        },
    )
    .map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&snapshot_frame, &snapshot, "assembled snapshot")?;

    let Some(report_frame) = report_frame else {
        return Ok(());
    };
    let _: ProjectReportConfig = replay_node_config(broker, &report_frame.node)?;
    let report_snapshot =
        replay_single_input_value::<PortfolioSnapshot>(broker, &report_frame.node)?;
    if report_snapshot != snapshot {
        return Err(replay_portfolio_mismatch(
            "ProjectReport did not consume the exact assembled snapshot output",
        ));
    }
    let report = mfm_state_portfolio::project_report_from_snapshot(report_snapshot)
        .map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&report_frame, &report, "project report")
}

fn replay_selected_candidate(
    broker: &replay::ReplayBroker,
    entry: &CollectedHoldingReceipt,
    config: &SelectHoldingsConfig,
    evidence: &FactQueryEvidence,
) -> replay::Result<(SelectedHolding, usize)> {
    let rows = fact_query_result_rows_from_receipt(evidence.receipt());
    if evidence.receipt().returned_field_summaries().is_none() {
        return Err(replay_portfolio_mismatch(
            "receipt-pinned query omitted returned field summaries",
        ));
    }
    let symbol = config
        .portfolio()
        .symbol_configs
        .iter()
        .find(|symbol| symbol.symbol_id.as_str() == entry.requirement().symbol_id)
        .ok_or_else(|| replay_portfolio_mismatch("receipt entry referenced an unknown symbol"))?;
    let mut matching = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let fact_ref = row.fact_ref();
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            replay_portfolio_mismatch("receipt-pinned query omitted metadata.store_commit_order")
        })?;
        let artifact = broker.retained_artifact(&fact_response_artifact_requirement(fact_ref))?;
        let candidate = match entry.source() {
            HoldingSourceKey::BitcoinNative { .. } => hydrate_btc_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                &artifact.artifact_bytes,
                store_commit_order,
            ),
            HoldingSourceKey::EvmNative { .. } => hydrate_evm_native_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                &artifact.artifact_bytes,
                store_commit_order,
            ),
            HoldingSourceKey::EvmErc20 { .. } => hydrate_evm_erc20_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                &artifact.artifact_bytes,
                store_commit_order,
            ),
        }
        .map_err(replay_adapter_error)?;
        if let Some(candidate) = candidate {
            matching.push((row_index, candidate));
        }
    }
    matching.sort_by(|left, right| {
        right
            .1
            .store_commit_order
            .cmp(&left.1.store_commit_order)
            .then_with(|| right.1.fact_claim_id.cmp(&left.1.fact_claim_id))
    });
    let Some((selected_index, candidate)) = matching.into_iter().next() else {
        return Err(replay_portfolio_mismatch(
            "receipt-pinned query had no exact fact-content identity match",
        ));
    };
    Ok((
        SelectedHolding {
            key: entry.requirement().clone(),
            anchor: candidate.anchor,
            store_commit_order: candidate.store_commit_order,
            fact_claim_id: candidate.fact_claim_id,
            material: candidate.response_material,
        },
        selected_index,
    ))
}

fn require_replay_exact_bounded_cardinality(
    evidence: &FactQueryEvidence,
    config: &SelectHoldingsConfig,
) -> replay::Result<()> {
    let rows = fact_query_result_rows_from_receipt(evidence.receipt());
    match evidence.receipt().result_cardinality() {
        QueryResultCardinality::Exact(count)
            if count == rows.len() as u64 && count <= config.candidate_bound() => {}
        QueryResultCardinality::AtLeast(count) if count >= config.candidate_scan_limit() => {
            return Err(replay_portfolio_mismatch(
                "candidate_bound_exhausted: receipt-pinned query saturated during replay",
            ));
        }
        _ => {
            return Err(replay_portfolio_mismatch(
                "receipt-pinned query did not retain an exact bounded result set",
            ));
        }
    }
    Ok(())
}

fn optional_replay_single_state_frame<S>(
    broker: &replay::ReplayBroker,
    label: &'static str,
) -> replay::Result<Option<replay::ProducedCellReplayFrame>>
where
    S: StateSpec,
{
    let kind = S::kind().map_err(replay_adapter_error)?;
    let version = S::version().map_err(replay_adapter_error)?;
    let mut frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == kind && node.state_version == version)
    })?;
    if frames.len() > 1 {
        return Err(replay_portfolio_mismatch(format!(
            "replay found more than one {label} output"
        )));
    }
    Ok(frames.pop())
}

fn replay_input_values<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<Vec<T>>
where
    T: MfmValue + DeserializeOwned,
{
    replay::produced_input_frames(
        broker,
        node,
        &T::semantic_id().map_err(replay_adapter_error)?,
        &T::schema_id().map_err(replay_adapter_error)?,
    )?
    .iter()
    .map(replay::decode_produced_value)
    .collect()
}

fn replay_single_input_value<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let values = replay_input_values::<T>(broker, node)?;
    if values.len() != 1 {
        return Err(replay_portfolio_mismatch(
            "certified receipt-pinned input did not contain exactly one value",
        ));
    }
    Ok(values.into_iter().next().expect("one input was checked"))
}

fn verify_replay_output_bytes<T: serde::Serialize>(
    frame: &replay::ProducedCellReplayFrame,
    expected: &T,
    label: &'static str,
) -> replay::Result<()> {
    if replay::canonical_value_bytes(expected)?.as_bytes() != frame.artifact_bytes {
        return Err(replay_portfolio_mismatch(format!(
            "portfolio {label} output did not match recomputed value"
        )));
    }
    Ok(())
}

fn replay_adapter_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_portfolio_mismatch(message: impl Into<String>) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}

/// Multiset plan match: first unmatched expected request with an equal query plan.
pub(crate) fn first_unmatched_plan_index(
    expected_requests: &[FactIndexReadRequest],
    matched_requirements: &BTreeSet<usize>,
    plan: &CanonicalFactQueryPlan,
) -> Option<usize> {
    expected_requests
        .iter()
        .enumerate()
        .find_map(|(index, request)| {
            (!matched_requirements.contains(&index) && request.plan() == plan).then_some(index)
        })
}
