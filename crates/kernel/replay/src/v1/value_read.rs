use super::*;

use mfm_values::{MfmConfig, MfmValue, ValidatedConfig};
use serde::{de::DeserializeOwned, Serialize};

/// Loads, JSON-decodes, and validates one certified node configuration artifact.
pub fn load_node_config<T>(broker: &ReplayBroker, node: &spec::NodeSpec) -> Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let evidence = store::ArtifactEvidenceRef {
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
        evidence_hash: evidence.evidence_hash()?,
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
    let config = serde_json::from_slice(&artifact.artifact_bytes).map_err(json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(|error| mismatch(error.to_string()))
}

/// Returns the one produced output frame for a certified state descriptor.
pub fn single_state_output_frame(
    broker: &ReplayBroker,
    state_kind: &StateKind,
    state_version: &StateVersion,
    label: &str,
) -> Result<ProducedCellReplayFrame> {
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == state_kind.clone() && node.state_version == state_version.clone())
    })?;
    if frames.len() != 1 {
        return Err(mismatch(format!(
            "replay requires exactly one {label} output"
        )));
    }
    Ok(frames
        .into_iter()
        .next()
        .expect("one replay frame was checked"))
}

/// Returns produced frames for every input cell with one value schema and semantic identity.
pub fn produced_input_frames(
    broker: &ReplayBroker,
    node: &spec::NodeSpec,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<Vec<ProducedCellReplayFrame>> {
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
            return Err(mismatch(
                "certified replay input did not have exactly one produced value",
            ));
        }
        let frame = matches.into_iter().next().expect("one replay input frame");
        if frame.cell.semantic_type_id != input_cell.semantic_type_id
            || frame.cell.schema_id != input_cell.schema_id
            || frame.cell.value_lineage != input_cell.value_lineage
        {
            return Err(mismatch(
                "certified replay input cell metadata did not match its produced value",
            ));
        }
        frames.push(frame);
    }
    Ok(frames)
}

/// Decodes one canonical produced-value artifact into its typed value.
pub fn decode_produced_value<T>(frame: &ProducedCellReplayFrame) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&frame.artifact_bytes).map_err(json_error)
}

/// Loads one retained external-read evidence artifact for a produced state attempt.
pub fn external_read_evidence<T>(
    broker: &ReplayBroker,
    frame: &ProducedCellReplayFrame,
) -> Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let schema_id = T::schema_id().map_err(|error| mismatch(error.to_string()))?;
    let references = broker
        .events()
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(reference)
                if reference.node_id.as_ref() == Some(&frame.produced.node_id)
                    && reference.attempt_id.as_ref() == Some(&frame.produced.attempt_id)
                    && reference.artifact_ref.role
                        == events::ArtifactRole::ExternalReadEvidence
                    && reference.artifact_ref.schema_id == schema_id =>
            {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if references.len() != 1 {
        return Err(mismatch(
            "replay external read did not have exactly one retained evidence artifact",
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
    serde_json::from_slice(&artifact.artifact_bytes).map_err(json_error)
}

/// Loads retained fact-query evidence recorded for one state attempt.
pub fn fact_query_evidence_for_attempt(
    broker: &ReplayBroker,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<Vec<mfm_facts::FactQueryEvidence>> {
    let mut evidence = Vec::new();
    for event in broker.events() {
        let events::KernelEventPayload::ArtifactReferenced(reference) = event.payload() else {
            continue;
        };
        if reference.artifact_ref.role != events::ArtifactRole::FactQueryEvidence
            || reference.node_id.as_ref() != Some(node_id)
            || reference.attempt_id.as_ref() != Some(attempt_id)
        {
            continue;
        }
        let requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ArtifactReferenced,
            artifact_id: reference.artifact_ref.artifact_id.clone(),
            evidence_hash: reference.artifact_ref.evidence_hash.clone(),
            digest: Some(reference.artifact_ref.content_digest.clone()),
            byte_len: Some(reference.artifact_ref.byte_len),
            media_type: Some(reference.artifact_ref.media_type.clone()),
            schema_id: Some(reference.artifact_ref.schema_id.clone()),
            semantic_type_id: reference.artifact_ref.semantic_type_id.clone(),
            producer_node_id: reference.node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(events::ArtifactRole::FactQueryEvidence),
        };
        let artifact = broker.retained_artifact(&requirement)?;
        evidence.push(
            mfm_facts::parse_canonical_fact_query_evidence_bytes(&artifact.artifact_bytes)
                .map_err(|error| mismatch(error.to_string()))?,
        );
    }
    Ok(evidence)
}

/// Verifies a typed fact output against its recorded fact claim and retained response artifact.
pub fn verify_recorded_fact_evidence<S, R>(
    broker: &ReplayBroker,
    frame: &ProducedCellReplayFrame,
    descriptor: &mfm_facts::FactDescriptor,
    subject: &S,
    response_value: &R,
) -> Result<()>
where
    S: Serialize,
    R: Serialize,
{
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
        return Err(mismatch(
            "replay fact output did not have exactly one FactRecorded event",
        ));
    }
    let claim = &records[0].claim;
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(descriptor).map_err(|error| mismatch(error.to_string()))?;
    if claim.fact_descriptor_hash() != &descriptor_hash
        || claim.fact_kind() != descriptor.fact_kind()
    {
        return Err(mismatch(
            "FactRecorded descriptor authority did not match the recomputed fact",
        ));
    }
    let subject_json = serde_json::to_value(subject).map_err(json_error)?;
    let expected_subject = mfm_facts::typed_fact_subject_evidence(descriptor, &subject_json)
        .map_err(|error| mismatch(error.to_string()))?;
    if claim.subject() != &expected_subject {
        return Err(mismatch(
            "FactRecorded subject authority did not match the recomputed fact",
        ));
    }
    let expected_bytes = canonical_value_bytes(response_value)?;
    let response = claim.response();
    if response.response_schema_id() != descriptor.response_schema_id()
        || response.response_hash() != &expected_bytes.content_digest()
    {
        return Err(mismatch(
            "FactRecorded response authority did not match the recomputed fact",
        ));
    }
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::FactResponse,
        artifact_id: response.artifact_id().clone(),
        evidence_hash: response.artifact_evidence_hash().clone(),
        digest: Some(response.response_hash().clone()),
        byte_len: Some(
            u64::try_from(expected_bytes.as_bytes().len())
                .map_err(|_| mismatch("replay fact response length overflowed u64"))?,
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
        return Err(mismatch(
            "FactRecorded response bytes did not match the recomputed fact",
        ));
    }
    Ok(())
}

/// Serializes a value to canonical JSON bytes for replay output comparison.
pub fn canonical_value_bytes<T>(value: &T) -> Result<mfm_canonical::PlainCanonicalJsonBytes>
where
    T: Serialize,
{
    let json = serde_json::to_string(value).map_err(json_error)?;
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).map_err(|error| {
        ReplayError::new(
            ReplayErrorKind::CertifiedEvidenceMismatch,
            error.to_string(),
        )
    })
}

fn collect_input_cells<'a>(
    input: &'a spec::InputBindingNodeSpec,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
    cells: &mut Vec<&'a spec::InputBindingCellSpec>,
) {
    match input {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => {
            if &cell.semantic_type_id == semantic_type_id && &cell.schema_id == schema_id {
                cells.push(cell);
            }
        }
        spec::InputBindingNodeSpec::Tuple(elements)
        | spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element, semantic_type_id, schema_id, cells);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells(&field.node, semantic_type_id, schema_id, cells);
            }
        }
    }
}

fn json_error(error: serde_json::Error) -> ReplayError {
    ReplayError::new(
        ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn mismatch(message: impl Into<String>) -> ReplayError {
    ReplayError::new(ReplayErrorKind::CertifiedEvidenceMismatch, message)
}
