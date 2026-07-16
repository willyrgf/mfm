//! Private evidence-only replay binding for the operation-local portfolio receipt fan-in.

use mfm_op_btc_collectors::BtcNetworkCollectionReceipt;
use mfm_op_evm_collectors::EvmNetworkCollectionReceipt;
use mfm_op_portfolio_snapshot::{
    assemble_portfolio_collection_receipt, AssemblePortfolioCollectionReceiptConfig,
    AssemblePortfolioCollectionReceiptInput, AssemblePortfolioCollectionReceiptState,
};
use mfm_program::StateSpec;
use mfm_replay::v1::{self as replay, load_node_config as replay_node_config};
use mfm_state_portfolio::PortfolioCollectionReceipt;
use mfm_values::MfmValue;
use serde::de::DeserializeOwned;

/// Rebuilds and verifies the operation-local exact portfolio collection receipt.
///
/// The app owns this private binding because it dispatches replay verification. The operation
/// remains planning-only and still owns the pure receipt semantics and graph topology; the
/// portfolio adapter verifies the receipt-pinned projection after this fan-in is established.
pub(crate) fn verify_portfolio_collection_receipt_replay(
    broker: &replay::ReplayBroker,
) -> replay::Result<PortfolioCollectionReceipt> {
    let state_kind =
        AssemblePortfolioCollectionReceiptState::kind().map_err(replay_binding_error)?;
    let state_version =
        AssemblePortfolioCollectionReceiptState::version().map_err(replay_binding_error)?;
    let frame = replay::single_state_output_frame(
        broker,
        &state_kind,
        &state_version,
        "portfolio collection receipt",
    )?;
    let config: AssemblePortfolioCollectionReceiptConfig = replay_node_config(broker, &frame.node)?;
    let bitcoin_receipts = replay_input_values::<BtcNetworkCollectionReceipt>(broker, &frame.node)?;
    let evm_receipts = replay_input_values::<EvmNetworkCollectionReceipt>(broker, &frame.node)?;
    let receipt = assemble_portfolio_collection_receipt(
        &config,
        AssemblePortfolioCollectionReceiptInput {
            bitcoin_receipts,
            evm_receipts,
        },
    )
    .map_err(replay_binding_error)?;
    verify_replay_output_bytes(&frame, &receipt, "portfolio collection receipt")?;
    Ok(receipt)
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
        &T::semantic_id().map_err(replay_binding_error)?,
        &T::schema_id().map_err(replay_binding_error)?,
    )?
    .iter()
    .map(replay::decode_produced_value)
    .collect()
}

fn verify_replay_output_bytes<T: serde::Serialize>(
    frame: &replay::ProducedCellReplayFrame,
    expected: &T,
    label: &'static str,
) -> replay::Result<()> {
    if replay::canonical_value_bytes(expected)?.as_bytes() != frame.artifact_bytes {
        return Err(replay_mismatch(format!(
            "portfolio {label} output did not match recomputed value"
        )));
    }
    Ok(())
}

fn replay_binding_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_mismatch(message: impl Into<String>) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
