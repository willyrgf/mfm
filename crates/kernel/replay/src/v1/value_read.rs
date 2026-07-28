use super::*;

use mfm_values::{MfmConfig, MfmValue, ValidatedConfig};
use serde::{de::DeserializeOwned, Serialize};

/// Loads, JSON-decodes, and validates one certified node configuration artifact.
pub fn load_node_config<T>(broker: &ReplayBroker, node: &spec::NodeSpec) -> Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = store::config_ref_artifact_requirement(&node.config_ref)?;
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
    frames
        .into_iter()
        .next()
        .ok_or_else(|| mismatch(format!("replay requires exactly one {label} output")))
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
        let frame = matches.into_iter().next().ok_or_else(|| {
            mismatch("certified replay input did not have exactly one produced value")
        })?;
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
        .artifact_references()?
        .into_iter()
        .filter(|reference| {
            reference.node_id.as_ref() == Some(&frame.produced.node_id)
                && reference.attempt_id.as_ref() == Some(&frame.produced.attempt_id)
                && reference.artifact_ref.role == events::ArtifactRole::ExternalReadEvidence
        })
        .collect::<Vec<_>>();
    if references.len() != 1 {
        return Err(mismatch(
            "replay external read did not have exactly one retained evidence artifact",
        ));
    }
    let reference = references[0];
    if reference.artifact_ref.schema_id != schema_id {
        return Err(mismatch(
            "replay external read evidence schema did not match the state contract",
        ));
    }
    let requirement = store::artifact_referenced_artifact_requirement(reference);
    let artifact = broker.retained_artifact(&requirement)?;
    serde_json::from_slice(&artifact.artifact_bytes).map_err(json_error)
}

/// Loads and decodes the complete certified input tree for one replayed node.
pub fn load_node_input<T>(broker: &ReplayBroker, node: &spec::NodeSpec) -> Result<T>
where
    T: mfm_values::StateInput + DeserializeOwned,
{
    let expected = T::input_schema_id().map_err(|error| mismatch(error.to_string()))?;
    if node.input_bindings.input_schema_id != expected {
        return Err(mismatch(format!(
            "replay input schema {} did not match state input schema {}",
            node.input_bindings.input_schema_id, expected
        )));
    }
    let value = replay_input_node_json(broker, node, &node.input_bindings.root)?;
    serde_json::from_value(value).map_err(json_error)
}

/// Materializes typed certified context authority for one replayed node.
pub fn load_node_context<C>(
    broker: &ReplayBroker,
    node: &spec::NodeSpec,
) -> Result<mfm_program::CertifiedContext<C>>
where
    C: mfm_program::StateContext,
{
    let context = match &node.context {
        spec::NodeContextSpec::NoContext => None,
        spec::NodeContextSpec::Required { context_ref } => Some(
            broker
                .certified_spec()
                .spec
                .contexts
                .iter()
                .find(|context| &context.context_ref == context_ref)
                .ok_or_else(|| mismatch("replay node referenced missing certified context"))?,
        ),
    };
    C::materialize_certified(context).map_err(|error| mismatch(error.to_string()))
}

/// Replays every produced output for one exact typed external-read descriptor.
///
/// This helper reconstructs config, arbitrary certified input trees, typed context, one primary
/// evidence artifact, and any auxiliary fact-query evidence before invoking the same state reducer
/// used by live execution.
#[allow(private_bounds)]
pub fn verify_external_read_state<S>(broker: &ReplayBroker) -> Result<()>
where
    S: mfm_program::ReadState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
    S::Evidence: DeserializeOwned,
    S::Facts: CompareReadFactBatch,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let descriptor =
        mfm_program::state_descriptor::<S>().map_err(|error| mismatch(error.to_string()))?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.descriptor_id == *descriptor.descriptor_id())
    })?;
    for frame in &frames {
        let config: S::Config = load_node_config(broker, &frame.node)?;
        let validated =
            ValidatedConfig::new(config).map_err(|error| mismatch(error.to_string()))?;
        let state = S::new(validated).map_err(|error| mismatch(error.to_string()))?;
        let input = load_node_input::<S::Input>(broker, &frame.node)?;
        let context = load_node_context::<S::Context>(broker, &frame.node)?;
        state
            .plan(&input, &context)
            .map_err(|error| mismatch(error.to_string()))?;
        let primary = external_read_evidence::<S::Evidence>(broker, frame)?;
        let fact_queries = fact_query_evidence_for_attempt(
            broker,
            &frame.produced.node_id,
            &frame.produced.attempt_id,
        )?;
        let evidence = mfm_program::ExternalReadEvidenceSet::new(primary, fact_queries);
        let (expected, facts) = state
            .reduce(&input, &evidence, &context)
            .map_err(|error| mismatch(error.to_string()))?;
        let expected_bytes = canonical_value_bytes(&expected)?;
        if frame.artifact_bytes != expected_bytes.as_bytes() {
            return Err(mismatch(
                "replayed external-read output did not match its retained evidence",
            ));
        }
        facts.compare(broker, frame)?;
    }
    Ok(())
}

/// Replays every produced output for one exact ordinary pure-state descriptor.
///
/// This helper reconstructs certified config, arbitrary input trees, and typed context before
/// invoking the same deterministic state behavior used by live execution. It performs no current
/// configuration, executable, transport, or capability IO.
pub fn verify_pure_state<S>(broker: &ReplayBroker) -> Result<()>
where
    S: mfm_program::PureState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
{
    let descriptor =
        mfm_program::state_descriptor::<S>().map_err(|error| mismatch(error.to_string()))?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.descriptor_id == *descriptor.descriptor_id())
    })?;
    for frame in &frames {
        let config: S::Config = load_node_config(broker, &frame.node)?;
        let validated =
            ValidatedConfig::new(config).map_err(|error| mismatch(error.to_string()))?;
        let state = S::new(validated).map_err(|error| mismatch(error.to_string()))?;
        let input = load_node_input::<S::Input>(broker, &frame.node)?;
        let context = load_node_context::<S::Context>(broker, &frame.node)?;
        let expected = state
            .run(input, &context)
            .map_err(|error| mismatch(error.to_string()))?;
        let expected_bytes = canonical_value_bytes(&expected)?;
        if frame.artifact_bytes != expected_bytes.as_bytes() {
            return Err(mismatch(
                "replayed pure-state output did not match deterministic state behavior",
            ));
        }
    }
    Ok(())
}

trait CompareReadFactBatch {
    fn compare(&self, broker: &ReplayBroker, frame: &ProducedCellReplayFrame) -> Result<()>;
}

impl CompareReadFactBatch for () {
    fn compare(&self, broker: &ReplayBroker, frame: &ProducedCellReplayFrame) -> Result<()> {
        if broker.fact_records()?.into_iter().any(|record| {
            record.payload.node_id == frame.produced.node_id
                && record.payload.attempt_id == frame.produced.attempt_id
        }) {
            return Err(mismatch(
                "fact-free read replay found unexpected recorded facts",
            ));
        }
        Ok(())
    }
}

impl<F> CompareReadFactBatch for mfm_values::NonEmpty<F>
where
    F: mfm_facts::MfmFactType,
{
    fn compare(&self, broker: &ReplayBroker, frame: &ProducedCellReplayFrame) -> Result<()> {
        verify_recorded_fact_batch_evidence(broker, frame, self.values())
    }
}

fn replay_input_node_json(
    broker: &ReplayBroker,
    consuming_node: &spec::NodeSpec,
    input: &spec::InputBindingNodeSpec,
) -> Result<serde_json::Value> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(serde_json::Value::Null),
        spec::InputBindingNodeSpec::Cell(cell) => {
            replay_input_cell_json(broker, consuming_node, cell)
        }
        spec::InputBindingNodeSpec::Tuple(elements)
        | spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => elements
            .iter()
            .map(|element| replay_input_node_json(broker, consuming_node, element))
            .collect::<Result<Vec<_>>>()
            .map(serde_json::Value::Array),
        spec::InputBindingNodeSpec::Struct(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.field_path.as_str().to_owned(),
                    replay_input_node_json(broker, consuming_node, &field.node)?,
                );
            }
            Ok(serde_json::Value::Object(object))
        }
    }
}

fn replay_input_cell_json(
    broker: &ReplayBroker,
    consuming_node: &spec::NodeSpec,
    input: &spec::InputBindingCellSpec,
) -> Result<serde_json::Value> {
    let certified = broker
        .certified_spec()
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == input.cell_id)
        .ok_or_else(|| mismatch("replay input cell was absent from the certified spec"))?;
    if certified.schema_id != input.schema_id
        || certified.semantic_type_id != input.semantic_type_id
        || certified.value_lineage != input.value_lineage
        || !replay_input_context_matches(&input.context, &certified.context)
    {
        return Err(mismatch(
            "replay input cell metadata did not match the certified cell",
        ));
    }
    if let spec::InputContextSpec::Required { context_ref, .. } = &input.context {
        if !matches!(
            &consuming_node.context,
            spec::NodeContextSpec::Required { context_ref: node_context } if node_context == context_ref
        ) {
            return Err(mismatch(
                "replay input context did not match the consuming node context",
            ));
        }
    }

    match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = broker
                .admission()?
                .seed_cells()
                .find(|seed| &seed.seed_id == seed_id && seed.cell_id == input.cell_id)
                .ok_or_else(|| mismatch("replay seed input evidence was missing"))?;
            let requirement = store::seed_cell_artifact_requirement(seed);
            let artifact = broker.retained_artifact(&requirement)?;
            serde_json::from_slice(&artifact.artifact_bytes).map_err(json_error)
        }
        spec::CellProducer::Node(producer_node_id) => {
            let frames = broker.produced_cell_frames_matching(|node, cell, _produced| {
                Ok(node.node_id == *producer_node_id && cell.cell_id == input.cell_id)
            })?;
            if frames.len() != 1 {
                return Err(mismatch(
                    "replay input cell did not have exactly one produced value",
                ));
            }
            let frame = frames.into_iter().next().ok_or_else(|| {
                mismatch("replay input cell did not have exactly one produced value")
            })?;
            serde_json::from_slice(&frame.artifact_bytes).map_err(json_error)
        }
    }
}

fn replay_input_context_matches(
    input: &spec::InputContextSpec,
    cell: &spec::CellContextSpec,
) -> bool {
    match (input, cell) {
        (spec::InputContextSpec::NoContext, spec::CellContextSpec::NoContext) => true,
        (
            spec::InputContextSpec::Required {
                context_ref,
                resource_kind,
                stage,
                producer,
            },
            spec::CellContextSpec::Bound {
                context_ref: cell_context_ref,
                resource_kind: cell_resource_kind,
                stage: cell_stage,
                producer: cell_producer,
            },
        ) => {
            context_ref == cell_context_ref
                && resource_kind == cell_resource_kind
                && stage == cell_stage
                && producer == cell_producer
        }
        _ => false,
    }
}

/// Loads retained fact-query evidence recorded for one state attempt.
pub fn fact_query_evidence_for_attempt(
    broker: &ReplayBroker,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<Vec<mfm_facts::FactQueryEvidence>> {
    let mut evidence = Vec::new();
    for reference in broker.artifact_references()? {
        if reference.artifact_ref.role != events::ArtifactRole::FactQueryEvidence
            || reference.node_id.as_ref() != Some(node_id)
            || reference.attempt_id.as_ref() != Some(attempt_id)
        {
            continue;
        }
        evidence.push(broker.verify_fact_query_evidence_reference(reference)?);
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
        .fact_records()?
        .into_iter()
        .filter(|record| {
            record.payload.node_id == frame.produced.node_id
                && record.payload.attempt_id == frame.produced.attempt_id
        })
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(mismatch(
            "replay fact output did not have exactly one FactRecorded event",
        ));
    }
    verify_fact_claim_evidence(
        broker,
        frame,
        descriptor,
        &records[0].payload.claim,
        subject,
        response_value,
    )
}

/// Verifies a homogeneous fact batch atomically recorded by one external-read settlement.
pub fn verify_recorded_fact_batch_evidence<F>(
    broker: &ReplayBroker,
    frame: &ProducedCellReplayFrame,
    facts: &[F],
) -> Result<()>
where
    F: mfm_facts::MfmFactType,
{
    let output_events = broker
        .produced_cell_records()?
        .into_iter()
        .filter(|record| {
            record.payload.node_id == frame.produced.node_id
                && record.payload.attempt_id == frame.produced.attempt_id
                && record.payload.cell_id == frame.produced.cell_id
        })
        .collect::<Vec<_>>();
    if output_events.len() != 1 {
        return Err(mismatch(
            "replay fact batch did not have exactly one matching CellProduced event",
        ));
    }
    let output_event = &output_events[0];
    let records = broker
        .fact_records()?
        .into_iter()
        .filter(|record| {
            record.payload.node_id == frame.produced.node_id
                && record.payload.attempt_id == frame.produced.attempt_id
        })
        .collect::<Vec<_>>();
    if records.len() != facts.len() {
        return Err(mismatch(
            "replay fact batch did not have exact FactRecorded coverage",
        ));
    }
    let descriptor = F::descriptor().map_err(|error| mismatch(error.to_string()))?;
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| mismatch(error.to_string()))?;
    for (fact, record) in facts.iter().zip(records.iter()) {
        let subject_json = serde_json::to_value(fact.subject()).map_err(json_error)?;
        let expected_subject = mfm_facts::typed_fact_subject_evidence(&descriptor, &subject_json)
            .map_err(|error| mismatch(error.to_string()))?;
        let expected_response = canonical_value_bytes(fact.response())?;
        let claim = &record.payload.claim;
        if claim.fact_descriptor_hash() != &descriptor_hash
            || claim.fact_kind() != descriptor.fact_kind()
            || claim.subject() != &expected_subject
            || claim.response().response_schema_id() != descriptor.response_schema_id()
            || claim.response().response_hash() != &expected_response.content_digest()
        {
            return Err(mismatch(
                "replay fact batch order or identity differed from reducer output",
            ));
        }
        if record.commit_key != output_event.commit_key || record.sequence != output_event.sequence
        {
            return Err(mismatch(
                "replay fact batch and state output were not recorded atomically",
            ));
        }
        verify_fact_claim_evidence(
            broker,
            frame,
            &descriptor,
            &record.payload.claim,
            fact.subject(),
            fact.response(),
        )?;
    }
    Ok(())
}

fn verify_fact_claim_evidence<S, R>(
    broker: &ReplayBroker,
    frame: &ProducedCellReplayFrame,
    descriptor: &mfm_facts::FactDescriptor,
    claim: &mfm_facts::FactClaim,
    subject: &S,
    response_value: &R,
) -> Result<()>
where
    S: Serialize,
    R: Serialize,
{
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
        byte_len: None,
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
