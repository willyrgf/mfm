use super::*;
use std::collections::BTreeMap;

use mfm_replay::v1::{
    self as replay, decode_produced_value as decode_replay_value,
    load_node_config as replay_node_config, produced_input_frames as replay_input_frames,
};
use mfm_states_btc::{BtcAddressBalanceObservation, BtcJointTip};
use mfm_values::{MfmValue, NonEmpty};

/// Verifies Bitcoin JSON-RPC observation cell outputs from retained capability read evidence.
pub fn verify_btc_jsonrpc_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    replay::verify_external_read_state::<ResolveBtcJointTipState>(broker)?;
    replay::verify_external_read_state::<ObserveBtcAddressBalanceState>(broker)?;

    let balance_kind = ObserveBtcAddressBalanceState::kind().map_err(replay_adapter_error)?;
    let balance_version = ObserveBtcAddressBalanceState::version().map_err(replay_adapter_error)?;
    let balance_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == balance_kind && node.state_version == balance_version)
    })?;
    verify_btc_address_balance_fact_replay(broker)?;
    verify_btc_shared_joint_tips(broker, &balance_frames)?;
    verify_btc_network_collection_receipt_replay(broker)?;
    Ok(())
}

fn verify_btc_shared_joint_tips(
    broker: &replay::ReplayBroker,
    frames: &[replay::ProducedCellReplayFrame],
) -> replay::Result<()> {
    let mut anchors = BTreeMap::<String, (u64, String)>::new();
    for frame in frames {
        let config: ObserveBtcAddressBalanceConfig = replay_node_config(broker, &frame.node)?;
        let tip = replay_joint_tip_input(broker, &frame.node)?;
        let anchor = (tip.block_height(), tip.block_hash().to_owned());
        if anchors
            .insert(config.network.clone(), anchor.clone())
            .is_some_and(|existing| existing != anchor)
        {
            return Err(replay_btc_mismatch(
                "Bitcoin same-network observations did not share one joint tip",
            ));
        }
    }
    Ok(())
}

fn verify_btc_address_balance_fact_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = RecordBtcAddressBalanceFactState::kind().map_err(replay_adapter_error)?;
    let state_version =
        RecordBtcAddressBalanceFactState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let observation_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcAddressBalanceObservation::semantic_id().map_err(replay_adapter_error)?,
            &BtcAddressBalanceObservation::schema_id().map_err(replay_adapter_error)?,
        )?;
        if observation_frames.len() != 1 {
            return Err(replay_btc_mismatch(
                "Bitcoin address-balance fact input was incomplete",
            ));
        }
        let observation: BtcAddressBalanceObservation =
            decode_replay_value(&observation_frames[0])?;
        let fact = observation.to_fact();
        ensure_canonical_value_matches(&fact, &frame.artifact_bytes)?;
        replay::verify_recorded_fact_evidence(
            broker,
            frame,
            &BtcAddressBalanceSnapshotFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_btc_network_collection_receipt_replay(
    broker: &replay::ReplayBroker,
) -> replay::Result<()> {
    let state_kind =
        AssembleBtcNetworkCollectionReceiptState::kind().map_err(replay_adapter_error)?;
    let state_version =
        AssembleBtcNetworkCollectionReceiptState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let _config: AssembleBtcNetworkCollectionReceiptConfig =
            replay_node_config(broker, &frame.node)?;
        let joint_tip_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcJointTip::semantic_id().map_err(replay_adapter_error)?,
            &BtcJointTip::schema_id().map_err(replay_adapter_error)?,
        )?;
        let fact_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcAddressBalanceSnapshotFact::semantic_id().map_err(replay_adapter_error)?,
            &BtcAddressBalanceSnapshotFact::schema_id().map_err(replay_adapter_error)?,
        )?;
        if joint_tip_frames.len() != 1 || fact_frames.is_empty() {
            return Err(replay_btc_mismatch(
                "Bitcoin network collection receipt inputs were incomplete",
            ));
        }
        let joint_tip: BtcJointTip = decode_replay_value(&joint_tip_frames[0])?;
        let facts = fact_frames
            .iter()
            .map(decode_replay_value)
            .collect::<replay::Result<Vec<BtcAddressBalanceSnapshotFact>>>()?;
        let input = AssembleBtcNetworkCollectionReceiptInput {
            joint_tip,
            balance_facts: NonEmpty::try_from_vec(facts).map_err(replay_adapter_error)?,
        };
        let expected =
            assemble_btc_network_collection_receipt(input).map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn replay_joint_tip_input(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<BtcJointTip> {
    let frames = replay_input_frames(
        broker,
        node,
        &BtcJointTip::semantic_id().map_err(replay_adapter_error)?,
        &BtcJointTip::schema_id().map_err(replay_adapter_error)?,
    )?;
    if frames.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin balance observation did not consume exactly one joint tip",
        ));
    }
    let resolve_kind = ResolveBtcJointTipState::kind().map_err(replay_adapter_error)?;
    let resolve_version = ResolveBtcJointTipState::version().map_err(replay_adapter_error)?;
    if frames[0].node.state_kind != resolve_kind || frames[0].node.state_version != resolve_version
    {
        return Err(replay_btc_mismatch(
            "Bitcoin balance observation input was not produced by joint-tip resolution",
        ));
    }
    decode_replay_value(&frames[0])
}

fn ensure_canonical_value_matches<T: serde::Serialize>(
    expected: &T,
    actual: &[u8],
) -> replay::Result<()> {
    if replay::canonical_value_bytes(expected)?.as_bytes() != actual {
        return Err(replay_btc_mismatch(
            "Bitcoin collector output did not match recomputed state output",
        ));
    }
    Ok(())
}

fn replay_adapter_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_btc_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
