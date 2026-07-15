use super::*;
use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_replay::v1 as replay;
use mfm_values::{MfmConfig, MfmValue, NonEmpty};
use serde::de::DeserializeOwned;

/// Rebuilds loaded checkpoint material from recorded query evidence and retained response bytes.
pub fn replay_loaded_checkpoint_from_evidence(
    config: &QueryCollectorCheckpointConfig,
    evidence: &mfm_facts::FactQueryEvidence,
    response: Option<CollectorCheckpointResponse>,
) -> Result<LoadedCollectorCheckpoint> {
    let request = config
        .request()
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)?;
    if evidence.plan() != request.plan() {
        return Err(BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
    }
    config
        .loaded_checkpoint_from_replay_evidence(evidence, response)
        .map_err(|_| BtcJsonRpcAdapterError::ReplayEvidenceMismatch)
}

/// Verifies Bitcoin JSON-RPC observation cell outputs from retained capability read evidence.
pub fn verify_btc_jsonrpc_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    verify_btc_collector_checkpoint_replay(broker)?;
    verify_btc_joint_tip_replay(broker)?;
    let chain_head_kind = ObserveBtcChainHeadState::kind().map_err(replay_adapter_error)?;
    let chain_head_version = ObserveBtcChainHeadState::version().map_err(replay_adapter_error)?;
    let chain_head_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == chain_head_kind && node.state_version == chain_head_version)
    })?;
    for frame in &chain_head_frames {
        verify_btc_chain_head_observation_replay(broker, frame)?;
    }

    let balance_kind = ObserveBtcAddressBalanceState::kind().map_err(replay_adapter_error)?;
    let balance_version = ObserveBtcAddressBalanceState::version().map_err(replay_adapter_error)?;
    let balance_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == balance_kind && node.state_version == balance_version)
    })?;
    for frame in &balance_frames {
        verify_btc_address_balance_observation_replay(broker, frame)?;
    }
    verify_btc_chain_head_fact_replay(broker)?;
    verify_btc_address_balance_fact_replay(broker)?;
    verify_btc_collector_checkpoint_fact_replay(broker)?;
    verify_btc_shared_joint_tips(broker, &balance_frames)?;
    verify_btc_network_collection_receipt_replay(broker)?;
    Ok(())
}

fn verify_btc_collector_checkpoint_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = QueryCollectorCheckpointState::kind().map_err(replay_adapter_error)?;
    let state_version = QueryCollectorCheckpointState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        verify_btc_collector_checkpoint_frame(broker, frame)?;
    }
    Ok(())
}

fn verify_btc_collector_checkpoint_frame(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: QueryCollectorCheckpointConfig = replay_node_config(broker, &frame.node)?;
    let evidences = replay_fact_query_evidence_for_attempt(
        broker,
        &frame.node.node_id,
        &frame.produced.attempt_id,
    )?;
    if evidences.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin collector checkpoint query must retain exactly one fact-query evidence",
        ));
    }
    let evidence = &evidences[0];
    let response = checkpoint_response_from_query_evidence(broker, evidence)?;
    let expected =
        replay_loaded_checkpoint_from_evidence(&config, evidence, response).map_err(|_| {
            replay_btc_mismatch("Bitcoin collector checkpoint replay evidence was rejected")
        })?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn checkpoint_response_from_query_evidence(
    broker: &replay::ReplayBroker,
    evidence: &mfm_facts::FactQueryEvidence,
) -> replay::Result<Option<CollectorCheckpointResponse>> {
    let rows = mfm_facts::fact_query_result_rows_from_receipt(evidence.receipt());
    let selected = evidence.selection().selected_indices();
    if selected.is_empty() {
        return Ok(None);
    }
    if selected.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin collector checkpoint selection was not a single index",
        ));
    }
    let index = selected[0] as usize;
    let row = rows.get(index).ok_or_else(|| {
        replay_btc_mismatch("Bitcoin collector checkpoint selection index was out of range")
    })?;
    let fact_ref = row.fact_ref();
    let requirement = fact_response_artifact_requirement(fact_ref);
    let artifact = broker.retained_artifact(&requirement)?;
    let response = hydrate_fact_response_json::<CollectorCheckpointResponse>(
        fact_ref,
        &artifact.artifact_bytes,
    )
    .map_err(|_| {
        replay_btc_mismatch(
            "Bitcoin collector checkpoint response could not be hydrated from retained evidence",
        )
    })?;
    Ok(Some(response))
}

fn replay_fact_query_evidence_for_attempt(
    broker: &replay::ReplayBroker,
    node_id: &mfm_ids::NodeId,
    attempt_id: &mfm_ids::AttemptId,
) -> replay::Result<Vec<mfm_facts::FactQueryEvidence>> {
    let mut evidence = Vec::new();
    for event in broker.events() {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::FactQueryEvidence
            || payload.node_id.as_ref() != Some(node_id)
            || payload.attempt_id.as_ref() != Some(attempt_id)
        {
            continue;
        }
        let requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ArtifactReferenced,
            artifact_id: payload.artifact_ref.artifact_id.clone(),
            evidence_hash: payload.artifact_ref.evidence_hash.clone(),
            digest: Some(payload.artifact_ref.content_digest.clone()),
            byte_len: Some(payload.artifact_ref.byte_len),
            media_type: Some(payload.artifact_ref.media_type.clone()),
            schema_id: Some(payload.artifact_ref.schema_id.clone()),
            semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
            producer_node_id: payload.node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(payload.artifact_ref.role),
        };
        let artifact = broker.retained_artifact(&requirement)?;
        let parsed = mfm_facts::parse_canonical_fact_query_evidence_bytes(&artifact.artifact_bytes)
            .map_err(replay_adapter_error)?;
        evidence.push(parsed);
    }
    Ok(evidence)
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

fn verify_btc_joint_tip_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = ResolveBtcJointTipState::kind().map_err(replay_adapter_error)?;
    let state_version = ResolveBtcJointTipState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let config: ResolveBtcJointTipConfig = replay_node_config(broker, &frame.node)?;
        let evidence: BtcChainHeadReadEvidence = replay_external_read_evidence(broker, frame)?;
        let selection = config.selection();
        if evidence.request_head_kind != selection.head_kind().as_str()
            || evidence.request_finality != selection.finality().as_str()
            || evidence.request_confirmation_depth != selection.finality().confirmation_depth()
            || evidence.response_head_kind != evidence.request_head_kind
            || evidence.response_finality != evidence.request_finality
            || evidence.response_confirmation_depth != evidence.request_confirmation_depth
            || evidence.network_id != config.network
            || evidence.source_identity != config.semantic_source_identity
            || evidence.bitcoin_network != config.bitcoin_network
            || evidence.observed_bitcoin_network != config.bitcoin_network
            || evidence.source_status != BtcSourceStatus::Synced.as_str()
        {
            return Err(replay_btc_mismatch(
                "Bitcoin joint-tip read evidence did not match certified request or source binding",
            ));
        }
        let expected = BtcJointTip::new(
            config.network,
            config.bitcoin_network,
            config.semantic_source_identity,
            evidence.block_height,
            evidence.block_hash,
            evidence.source_status,
            evidence.observed_bitcoin_network,
        )
        .map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&expected, &frame.artifact_bytes)?;
    }
    Ok(())
}

fn verify_btc_chain_head_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcChainHeadConfig = replay_node_config(broker, &frame.node)?;
    let request = chain_head_request(&config).map_err(replay_adapter_error)?;
    let evidence: BtcChainHeadReadEvidence = replay_external_read_evidence(broker, frame)?;
    let output: BtcChainHeadObservation =
        serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    let selection = request.selection();
    if evidence.request_head_kind != selection.head_kind().as_str()
        || evidence.request_finality != selection.finality().as_str()
        || evidence.request_confirmation_depth != selection.finality().confirmation_depth()
        || evidence.response_head_kind != evidence.request_head_kind
        || evidence.response_finality != evidence.request_finality
        || evidence.response_confirmation_depth != evidence.request_confirmation_depth
        || evidence.network_id != config.network
        || evidence.source_identity != config.semantic_source_identity
        || evidence.bitcoin_network != config.bitcoin_network
        || evidence.observed_bitcoin_network != config.bitcoin_network
        || evidence.source_status != output.response().observed_source_status()
        || output.source_read_count() != 1
        || output.subject().network() != evidence.network_id
        || output.subject().bitcoin_network() != evidence.bitcoin_network
        || output.subject().semantic_source_identity() != evidence.source_identity
        || output.subject().head_kind() != evidence.response_head_kind
        || output.response().block_height() != evidence.block_height
        || output.response().block_hash() != evidence.block_hash
        || output.response().observed_bitcoin_network() != evidence.observed_bitcoin_network
        || output.response().finality_policy() != evidence.response_finality
        || output.response().confirmation_depth() != evidence.response_confirmation_depth
        || output.response().provider_time_unix_ms() != evidence.provider_time_unix_ms
    {
        return Err(replay_btc_mismatch(
            "Bitcoin chain-head output did not match retained capability read evidence",
        ));
    }
    Ok(())
}

fn verify_btc_address_balance_observation_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let config: ObserveBtcAddressBalanceConfig = replay_node_config(broker, &frame.node)?;
    let input_tip = replay_joint_tip_input(broker, &frame.node)?;
    let binding = address_balance_binding(&config).map_err(replay_adapter_error)?;
    let evidence: BtcBalanceReadEvidence = replay_external_read_evidence(broker, frame)?;
    let request = address_balance_request(&config, &input_tip).map_err(replay_adapter_error)?;
    if evidence.request_address != request.address().as_str()
        || evidence.request_block_height != request.block_height()
        || evidence.request_block_hash != request.block_hash().as_str()
        || evidence.response_address != evidence.request_address
        || evidence.response_block_height != evidence.request_block_height
        || evidence.response_block_hash != evidence.request_block_hash
        || evidence.network_id != config.network
        || evidence.source_identity != config.semantic_source_identity
        || evidence.bitcoin_network != config.bitcoin_network
        || evidence.observed_bitcoin_network != config.bitcoin_network
        || evidence.source_status != BtcSourceStatus::Synced.as_str()
    {
        return Err(replay_btc_mismatch(
            "Bitcoin address-balance read evidence did not match certified request or source binding",
        ));
    }
    let capability_response = BtcBalanceReadResponse {
        evidence: RedactedBtcSourceEvidence::from_binding(
            &binding,
            config.bitcoin_network.clone(),
            BtcSourceStatus::Synced,
        )
        .map_err(replay_adapter_error)?,
        address: BtcAddress::new(&evidence.response_address).map_err(replay_adapter_error)?,
        balance_sats: evidence.response_balance_sats,
        block_height: evidence.response_block_height,
        block_hash: BtcBlockHash::new(&evidence.response_block_hash)
            .map_err(replay_adapter_error)?,
    };
    let expected =
        normalize_btc_address_balance_observation(&config, &input_tip, &capability_response)
            .map_err(replay_adapter_error)?;
    ensure_canonical_value_matches(&expected, &frame.artifact_bytes)
}

fn verify_btc_chain_head_fact_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let state_kind = RecordBtcChainHeadFactState::kind().map_err(replay_adapter_error)?;
    let state_version = RecordBtcChainHeadFactState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let observation_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcChainHeadObservation::semantic_id().map_err(replay_adapter_error)?,
            &BtcChainHeadObservation::schema_id().map_err(replay_adapter_error)?,
        )?;
        if observation_frames.len() != 1 {
            return Err(replay_btc_mismatch(
                "Bitcoin chain-head fact input was incomplete",
            ));
        }
        let observation: BtcChainHeadObservation = decode_replay_value(&observation_frames[0])?;
        let fact = observation.to_fact();
        ensure_canonical_value_matches(&fact, &frame.artifact_bytes)?;
        verify_recorded_fact_evidence(
            broker,
            frame,
            &BtcChainHeadFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
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
        verify_recorded_fact_evidence(
            broker,
            frame,
            &BtcAddressBalanceSnapshotFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_btc_collector_checkpoint_fact_replay(
    broker: &replay::ReplayBroker,
) -> replay::Result<()> {
    let state_kind = RecordCollectorCheckpointState::kind().map_err(replay_adapter_error)?;
    let state_version = RecordCollectorCheckpointState::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind && node.state_version == state_version)
    })?;
    for frame in &frames {
        let config: mfm_states_btc::RecordCollectorCheckpointConfig =
            replay_node_config(broker, &frame.node)?;
        let chain_head_frames = replay_input_frames(
            broker,
            &frame.node,
            &BtcChainHeadFact::semantic_id().map_err(replay_adapter_error)?,
            &BtcChainHeadFact::schema_id().map_err(replay_adapter_error)?,
        )?;
        let loaded_checkpoint_frames = replay_input_frames(
            broker,
            &frame.node,
            &LoadedCollectorCheckpoint::semantic_id().map_err(replay_adapter_error)?,
            &LoadedCollectorCheckpoint::schema_id().map_err(replay_adapter_error)?,
        )?;
        if chain_head_frames.len() != 1 || loaded_checkpoint_frames.len() != 1 {
            return Err(replay_btc_mismatch(
                "Bitcoin collector checkpoint fact inputs were incomplete",
            ));
        }
        let chain_head_fact: BtcChainHeadFact = decode_replay_value(&chain_head_frames[0])?;
        let loaded_checkpoint: LoadedCollectorCheckpoint =
            decode_replay_value(&loaded_checkpoint_frames[0])?;
        let fact =
            record_collector_checkpoint_from_outputs(&config, &chain_head_fact, &loaded_checkpoint)
                .map_err(replay_adapter_error)?;
        ensure_canonical_value_matches(&fact, &frame.artifact_bytes)?;
        verify_recorded_fact_evidence(
            broker,
            frame,
            &CollectorCheckpointFact::descriptor().map_err(replay_adapter_error)?,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_recorded_fact_evidence<S: serde::Serialize, R: serde::Serialize>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    descriptor: &mfm_facts::FactDescriptor,
    subject: &S,
    response_value: &R,
) -> replay::Result<()> {
    let records = broker
        .events()
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::FactRecorded(payload)
                if payload.node_id == frame.produced.node_id
                    && payload.attempt_id == frame.produced.attempt_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin fact output did not have exactly one FactRecorded event",
        ));
    }
    let claim = &records[0].claim;
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(descriptor).map_err(replay_adapter_error)?;
    if claim.fact_descriptor_hash() != &descriptor_hash
        || claim.fact_kind() != descriptor.fact_kind()
    {
        return Err(replay_btc_mismatch(
            "Bitcoin FactRecorded descriptor authority did not match the recomputed fact",
        ));
    }
    let subject_json = serde_json::to_value(subject).map_err(replay_json_error)?;
    let expected_subject = mfm_facts::typed_fact_subject_evidence(descriptor, &subject_json)
        .map_err(replay_adapter_error)?;
    if claim.subject() != &expected_subject {
        return Err(replay_btc_mismatch(
            "Bitcoin FactRecorded subject authority did not match the recomputed fact",
        ));
    }
    let expected_bytes = canonical_json_bytes(response_value)?;
    let response = claim.response();
    if response.response_schema_id() != descriptor.response_schema_id()
        || response.response_hash() != &expected_bytes.content_digest()
    {
        return Err(replay_btc_mismatch(
            "Bitcoin FactRecorded response authority did not match the recomputed fact",
        ));
    }
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::FactResponse,
        artifact_id: response.artifact_id().clone(),
        evidence_hash: response.artifact_evidence_hash().clone(),
        digest: Some(response.response_hash().clone()),
        byte_len: Some(
            u64::try_from(expected_bytes.as_bytes().len())
                .map_err(|_| replay_btc_mismatch("Bitcoin fact response length overflowed u64"))?,
        ),
        media_type: None,
        schema_id: Some(response.response_schema_id().clone()),
        semantic_type_id: None,
        producer_node_id: Some(frame.produced.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::FactResponse),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    if artifact.artifact_bytes != expected_bytes.as_bytes() {
        return Err(replay_btc_mismatch(
            "Bitcoin FactRecorded response bytes did not match the recomputed fact",
        ));
    }
    Ok(())
}

fn canonical_json_bytes<T: serde::Serialize>(value: &T) -> replay::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(replay_json_error)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(replay_adapter_error)
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
            return Err(replay_btc_mismatch(
                "certified collector input cell did not have exactly one produced value",
            ));
        }
        let frame = matches.into_iter().next().expect("one frame");
        if frame.cell.semantic_type_id != input_cell.semantic_type_id
            || frame.cell.schema_id != input_cell.schema_id
            || frame.cell.value_lineage != input_cell.value_lineage
        {
            return Err(replay_btc_mismatch(
                "collector input cell metadata did not match its produced value",
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

fn replay_external_read_evidence<T>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let references = broker
        .events()
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(reference)
                if reference.node_id.as_ref() == Some(&frame.produced.node_id)
                    && reference.attempt_id.as_ref() == Some(&frame.produced.attempt_id)
                    && reference.artifact_ref.role
                        == events::ArtifactRole::ExternalReadEvidence
                    && reference.artifact_ref.schema_id == T::schema_id().ok()? =>
            {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if references.len() != 1 {
        return Err(replay_btc_mismatch(
            "Bitcoin external read did not have exactly one retained evidence artifact",
        ));
    }
    let reference = references[0];
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference.artifact_ref.artifact_id.clone(),
        evidence_hash: reference.artifact_ref.evidence_hash.clone(),
        digest: Some(reference.artifact_ref.content_digest.clone()),
        byte_len: Some(reference.artifact_ref.byte_len),
        media_type: Some(reference.artifact_ref.media_type.clone()),
        schema_id: Some(reference.artifact_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(frame.produced.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::ExternalReadEvidence),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)
}

fn decode_replay_value<T>(frame: &replay::ProducedCellReplayFrame) -> replay::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)
}

fn ensure_canonical_value_matches<T: serde::Serialize>(
    expected: &T,
    actual: &[u8],
) -> replay::Result<()> {
    let json = serde_json::to_string(expected).map_err(replay_json_error)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(replay_adapter_error)?;
    if canonical.as_bytes() != actual {
        return Err(replay_btc_mismatch(
            "Bitcoin collector output did not match recomputed state output",
        ));
    }
    Ok(())
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
            .map_err(replay_adapter_error)?,
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
        .map_err(replay_adapter_error)
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

fn replay_btc_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
