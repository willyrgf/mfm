use super::*;

use std::collections::BTreeMap;

use mfm_replay::v1::{self as replay, load_node_config as replay_node_config};

/// Verifies Bitcoin selection and direct EVM snapshot projection from retained evidence only.
pub fn verify_portfolio_replay(
    broker: &replay::ReplayBroker,
    receipt: &PortfolioCollectionReceipt,
) -> replay::Result<()> {
    replay::verify_external_read_state::<SelectHoldingsState>(broker)?;
    replay::verify_external_read_state::<CollectEvmNetworkState>(broker)?;
    let published_evm_snapshots = verify_published_evm_snapshots(broker)?;

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
    let select_input = replay::load_node_input::<SelectHoldingsInput>(broker, &select_frame.node)?;
    if select_input.receipt != *receipt {
        return Err(replay_portfolio_mismatch(
            "SelectHoldings did not consume the exact assembled collection receipt",
        ));
    }
    validate_receipt_against_portfolio(receipt, config.portfolio())
        .map_err(replay_adapter_error)?;
    let selected_output: SelectedHoldings =
        serde_json::from_slice(&select_frame.artifact_bytes).map_err(replay_json_error)?;

    let Some(snapshot_frame) = snapshot_frame else {
        return Ok(());
    };
    let snapshot_config: AssembleSnapshotConfig = replay_node_config(broker, &snapshot_frame.node)?;
    if snapshot_config.portfolio() != config.portfolio() {
        return Err(replay_portfolio_mismatch(
            "portfolio snapshot config does not match SelectHoldings config",
        ));
    }
    let snapshot_input =
        replay::load_node_input::<AssembleSnapshotInput>(broker, &snapshot_frame.node)?;
    if snapshot_input.receipt != *receipt || snapshot_input.holdings != selected_output {
        return Err(replay_portfolio_mismatch(
            "AssembleSnapshot did not consume the exact selected holdings and receipt",
        ));
    }
    let mut replay_snapshot_by_network = snapshot_input
        .evm_snapshots
        .iter()
        .map(|snapshot| (snapshot.network_id(), snapshot))
        .collect::<BTreeMap<_, _>>();
    if replay_snapshot_by_network.len() != snapshot_input.evm_snapshots.len()
        || replay_snapshot_by_network.len() != published_evm_snapshots.len()
    {
        return Err(replay_portfolio_mismatch(
            "AssembleSnapshot EVM snapshot coverage was not exact",
        ));
    }
    for published in &published_evm_snapshots {
        if replay_snapshot_by_network.remove(published.network_id()) != Some(published) {
            return Err(replay_portfolio_mismatch(
                "AssembleSnapshot did not consume exact published EVM snapshots",
            ));
        }
    }
    let snapshot =
        assemble_snapshot(&snapshot_config, snapshot_input).map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&snapshot_frame, &snapshot, "assembled snapshot")?;

    let Some(report_frame) = report_frame else {
        return Ok(());
    };
    let _: ProjectReportConfig = replay_node_config(broker, &report_frame.node)?;
    let report_input = replay::load_node_input::<ProjectReportInput>(broker, &report_frame.node)?;
    if report_input.snapshot != snapshot {
        return Err(replay_portfolio_mismatch(
            "ProjectReport did not consume the exact assembled snapshot output",
        ));
    }
    let report = mfm_state_portfolio::project_report_from_snapshot(report_input.snapshot)
        .map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&report_frame, &report, "project report")
}

fn verify_published_evm_snapshots(
    broker: &replay::ReplayBroker,
) -> replay::Result<Vec<mfm_state_portfolio::EvmNetworkSnapshot>> {
    let kind = PublishEvmHoldingsState::kind().map_err(replay_adapter_error)?;
    let version = PublishEvmHoldingsState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == kind && node.state_version == version)
    })?;
    let mut snapshots = Vec::with_capacity(frames.len());
    for frame in frames {
        let config: EvmNetworkCollectionConfig = replay_node_config(broker, &frame.node)?;
        let input = replay::load_node_input::<PublishEvmHoldingsInput>(broker, &frame.node)?;
        let snapshot = mfm_state_portfolio::publish_evm_holdings(&config, input.batch)
            .map_err(replay_adapter_error)?;
        verify_replay_output_bytes(&frame, &snapshot, "published EVM network snapshot")?;
        replay::verify_recorded_fact_batch_evidence(broker, &frame, &snapshot.facts())?;
        snapshots.push(snapshot);
    }
    snapshots.sort_by(|left, right| left.network_id().cmp(right.network_id()));
    if snapshots
        .windows(2)
        .any(|pair| pair[0].network_id() == pair[1].network_id())
    {
        return Err(replay_portfolio_mismatch(
            "replay found duplicate published EVM network snapshots",
        ));
    }
    Ok(snapshots)
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

fn replay_json_error(error: serde_json::Error) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
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
