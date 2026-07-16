//! Evidence-only replay verification for the operation-local collection receipt.

use super::*;

use mfm_events::v1 as events;
use mfm_replay::v1 as replay;
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;

/// Rebuilds and verifies the operation-local exact portfolio collection receipt.
///
/// This stays with the operation because only the operation owns the logical manifest and family
/// receipt fan-in. The portfolio adapter receives the verified receipt afterward and verifies
/// receipt-pinned fact selection, snapshot assembly, and report projection.
pub fn verify_portfolio_collection_receipt_replay(
    broker: &replay::ReplayBroker,
) -> replay::Result<PortfolioCollectionReceipt> {
    let frame = replay_single_state_frame::<AssemblePortfolioCollectionReceiptState>(
        broker,
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
    .map_err(replay_operation_error)?;
    verify_replay_output_bytes(&frame, &receipt, "portfolio collection receipt")?;
    Ok(receipt)
}

fn replay_single_state_frame<S>(
    broker: &replay::ReplayBroker,
    label: &'static str,
) -> replay::Result<replay::ProducedCellReplayFrame>
where
    S: StateSpec,
{
    let kind = S::kind().map_err(replay_operation_error)?;
    let version = S::version().map_err(replay_operation_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == kind && node.state_version == version)
    })?;
    if frames.len() != 1 {
        return Err(replay_mismatch(format!(
            "portfolio receipt replay requires exactly one {label} output"
        )));
    }
    Ok(frames
        .into_iter()
        .next()
        .expect("one replay frame was checked"))
}

fn replay_input_values<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<Vec<T>>
where
    T: MfmValue + DeserializeOwned,
{
    replay_input_frames(
        broker,
        node,
        &T::semantic_id().map_err(replay_operation_error)?,
        &T::schema_id().map_err(replay_operation_error)?,
    )?
    .iter()
    .map(decode_replay_value)
    .collect()
}

fn replay_input_frames(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
    semantic_type_id: &mfm_ids::SemanticTypeId,
    schema_id: &mfm_ids::SchemaId,
) -> replay::Result<Vec<replay::ProducedCellReplayFrame>> {
    let mut cells = Vec::new();
    collect_input_cells(
        &node.input_bindings.root,
        semantic_type_id,
        schema_id,
        &mut cells,
    );
    let mut frames = Vec::with_capacity(cells.len());
    for input_cell in cells {
        let matches = broker.produced_cell_frames_matching(|_node, cell, _produced| {
            Ok(cell.cell_id == input_cell.cell_id)
        })?;
        if matches.len() != 1 {
            return Err(replay_mismatch(
                "certified portfolio receipt input did not have exactly one produced value",
            ));
        }
        let frame = matches.into_iter().next().expect("one replay input frame");
        if frame.cell.semantic_type_id != input_cell.semantic_type_id
            || frame.cell.schema_id != input_cell.schema_id
            || frame.cell.value_lineage != input_cell.value_lineage
        {
            return Err(replay_mismatch(
                "portfolio receipt input cell metadata did not match its produced value",
            ));
        }
        frames.push(frame);
    }
    Ok(frames)
}

fn collect_input_cells<'a>(
    input: &'a mfm_spec::v1::InputBindingNodeSpec,
    semantic_type_id: &mfm_ids::SemanticTypeId,
    schema_id: &mfm_ids::SchemaId,
    cells: &mut Vec<&'a mfm_spec::v1::InputBindingCellSpec>,
) {
    match input {
        mfm_spec::v1::InputBindingNodeSpec::Unit => {}
        mfm_spec::v1::InputBindingNodeSpec::Cell(cell) => {
            if &cell.semantic_type_id == semantic_type_id && &cell.schema_id == schema_id {
                cells.push(cell);
            }
        }
        mfm_spec::v1::InputBindingNodeSpec::Tuple(elements)
        | mfm_spec::v1::InputBindingNodeSpec::Vec { elements, .. }
        | mfm_spec::v1::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element, semantic_type_id, schema_id, cells);
            }
        }
        mfm_spec::v1::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells(&field.node, semantic_type_id, schema_id, cells);
            }
        }
    }
}

fn decode_replay_value<T>(frame: &replay::ProducedCellReplayFrame) -> replay::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)
}

fn replay_node_config<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let config_evidence = store::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        evidence_hash: config_evidence
            .evidence_hash()
            .map_err(replay_operation_error)?,
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    let config: T = serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(replay_operation_error)
}

fn verify_replay_output_bytes<T: serde::Serialize>(
    frame: &replay::ProducedCellReplayFrame,
    expected: &T,
    label: &'static str,
) -> replay::Result<()> {
    let json = serde_json::to_string(expected).map_err(replay_json_error)?;
    let bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(replay_operation_error)?;
    if bytes.as_bytes() != frame.artifact_bytes {
        return Err(replay_mismatch(format!(
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

fn replay_operation_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_mismatch(message: impl Into<String>) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
