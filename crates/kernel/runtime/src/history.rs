use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, CellId, ContentDigest, NodeId, RunId, SpecHash};
use mfm_manual_auth::manual_authorization_proof_schema_id;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{artifact_role_name, staged_artifact_binding_kind, verify_artifact_bytes};
use crate::commit::SealedTerminalCommitValidation;
use crate::error::async_store_error;
use crate::framework::{
    bootstrap_run_receipt_artifact, build_retention_manifest_artifact_with_producer,
    certified_bootstrap_run_node, certified_complete_run_node,
    certified_resolve_saga_terminal_node, certified_retention_manifest_node,
    complete_run_receipt_json, projected_retention_manifest, public_output_receipt_digest,
    public_output_rendered_digest, resolve_saga_terminal_receipt_json,
    retention_manifest_receipt_json, run_completion_evidence, saga_terminal_completion_outcome,
    GenesisContext,
};
use crate::side_effects::{
    side_effect_payload_ref, validate_atomic_side_effect_failure_pairs,
    validate_historical_side_effect_confirmation, validate_historical_side_effect_failure,
    validate_historical_side_effect_payload, validate_recovery_frontier,
    HistoricalSideEffectLedger,
};
use crate::{
    attempt_id, config_artifact_reference_payloads, config_ref_key, require_adapter,
    require_capability, retention_ref_for_artifact, validate_public_output,
    validate_public_output_render_node, CertifiedRuntimeCapabilities, CertifiedRuntimeSpec,
    MaterializedCell, MaterializedCellTerminal, MaterializedInputNode, MaterializedInputs,
    NamedMaterializedInput, RecordedFact, RecordedFacts, Result, RuntimeError,
};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeRunView {
    pub(crate) stream: Vec<store::KernelEventEnvelope>,
    pub(crate) projections: store::ProjectionSnapshot,
    pub(crate) seed_cells: BTreeMap<CellId, events::SeedCellRef>,
    pub(crate) config_artifacts: BTreeMap<String, store::ArtifactEvidenceRef>,
    pub(crate) artifact_refs: BTreeMap<ArtifactId, CommittedArtifactReference>,
    pub(crate) next_seq: store::StreamSeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedArtifactReference {
    pub(crate) evidence: store::ArtifactEvidenceRef,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) commit_seq: store::StreamSeq,
    pub(crate) commit_key: store::CommitKey,
}

/// Store-owned run stream validated against certified runtime authority.
#[derive(Debug, Clone)]
pub struct VerifiedRunStream {
    run_id: RunId,
    spec_hash: SpecHash,
    committed: store::CommittedRunStream,
}

impl VerifiedRunStream {
    /// Loads the authoritative run stream from a typed store and validates it against certified
    /// runtime authority.
    pub fn from_store<S>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self>
    where
        S: store::TypedRunEventStore + ?Sized,
    {
        let committed =
            store::CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(run_id))?;
        Self::from_committed_stream(runtime_spec, committed)
    }

    /// Loads the authoritative run stream from an async typed store and validates it against
    /// certified runtime authority.
    pub async fn from_async_store<S>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self>
    where
        S: store::AsyncTypedRunEventStore + ?Sized,
    {
        let committed = store::CommittedRunStream::from_events(
            run_id.clone(),
            store
                .load_run_stream(run_id)
                .await
                .map_err(async_store_error)?,
        )?;
        Self::from_committed_stream(runtime_spec, committed)
    }

    pub(crate) fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        let committed = store::CommittedRunStream::from_events(run_id.clone(), stream.to_vec())?;
        Self::from_committed_stream(runtime_spec, committed)
    }

    pub(crate) fn from_committed_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: store::CommittedRunStream,
    ) -> Result<Self> {
        RuntimeRunView::from_committed_stream(runtime_spec, &committed)?;
        Ok(Self {
            run_id: committed.run_id().clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            committed,
        })
    }

    /// Run id covered by this verified stream.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Certified spec hash covered by this verified stream.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Authoritative committed event envelopes covered by this verified stream.
    pub fn events(&self) -> &[store::KernelEventEnvelope] {
        self.committed.events()
    }

    /// Projection rebuilt from the verified committed stream.
    pub fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.committed.projection()
    }

    /// Store-owned stream authority covered by this runtime verification.
    pub fn committed_stream(&self) -> &store::CommittedRunStream {
        &self.committed
    }
}

impl RuntimeRunView {
    pub(crate) fn from_store<S: store::TypedRunEventStore + ?Sized>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self> {
        let committed =
            store::CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(run_id))?;
        Self::from_committed_stream(runtime_spec, &committed)
    }

    pub(crate) fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        let committed = store::CommittedRunStream::from_events(run_id.clone(), stream.to_vec())?;
        Self::from_committed_stream(runtime_spec, &committed)
    }

    pub(crate) fn from_committed_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: &store::CommittedRunStream,
    ) -> Result<Self> {
        let stream = committed.events();
        let projections = committed.projection().clone();
        let mut run_started = None;
        for event in stream {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    if &payload.run_id != committed.run_id() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "run stream contains RunStarted for {} while executing {}",
                            payload.run_id,
                            committed.run_id()
                        )));
                    }
                    if &payload.spec_hash != runtime_spec.spec_hash() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "RunStarted spec hash {} does not match certified {}",
                            payload.spec_hash,
                            runtime_spec.spec_hash()
                        )));
                    }
                    if run_started.replace(payload.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(
                            "run stream contains multiple RunStarted events".to_owned(),
                        ));
                    }
                }
                payload if payload_spec_hash(payload) != *runtime_spec.spec_hash() => {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "event payload spec hash {} does not match certified {}",
                        payload_spec_hash(payload),
                        runtime_spec.spec_hash()
                    )));
                }
                _ => {}
            }
        }
        let Some(run_started) = run_started else {
            return Err(RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            ));
        };
        validate_historical_run_stream(runtime_spec, committed.run_id(), stream, &projections)?;
        let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
        let config_artifacts = config_artifacts_from_stream(runtime_spec, stream)?;
        let artifact_refs = artifact_refs_from_stream(stream)?;
        Ok(Self {
            stream: stream.to_vec(),
            projections,
            seed_cells,
            config_artifacts,
            artifact_refs,
            next_seq: committed.next_seq(),
        })
    }
}

/// Validates a stored typed run stream against the certified runtime spec without executing work.
///
/// This is the read-only counterpart to scheduler resume: callers that inspect, render, or
/// append-only resume a run must still prove the historical stream is bound to the stored certified
/// spec before trusting projections.
pub fn validate_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    VerifiedRunStream::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

pub(crate) fn recorded_facts_for_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<RecordedFacts> {
    let mut facts = BTreeMap::new();
    for ((fact_node_id, fact_attempt_id, fact_key), projection) in projections.facts() {
        if fact_node_id != node_id || fact_attempt_id != attempt_id {
            continue;
        }
        if projection.node_id != *node_id
            || projection.attempt_id != *attempt_id
            || projection.fact_key != *fact_key
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact projection {} for node {} attempt {} is internally inconsistent",
                fact_key, node_id, attempt_id
            )));
        }
        facts.insert(
            fact_key.clone(),
            RecordedFact {
                fact_key: fact_key.clone(),
                request_schema_id: projection.request_schema_id.clone(),
                request_hash: projection.request_hash.clone(),
                response_schema_id: projection.response_schema_id.clone(),
                response_hash: projection.response_hash.clone(),
                artifact_id: projection.artifact_id.clone(),
                capability_kind: projection.capability_kind.clone(),
                capability_version: projection.capability_version.clone(),
                adapter_kind: projection.adapter_kind.clone(),
                adapter_version: projection.adapter_version.clone(),
            },
        );
    }
    Ok(RecordedFacts { facts })
}

pub(crate) fn materialize_inputs(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputs> {
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, &node.input_bindings.root, view)?,
    })
}

fn materialize_input_node(
    runtime_spec: &CertifiedRuntimeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputNode> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, cell, view)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, &field.node, view)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, element, view))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    cell: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedCell> {
    let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {} is not certified by the spec",
            cell.cell_id
        ))
    })?;
    if certified.schema_id != cell.schema_id
        || certified.semantic_type_id != cell.semantic_type_id
        || certified.value_lineage != cell.value_lineage
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} metadata does not match certified cell",
            cell.cell_id
        )));
    }
    let terminal = match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = view.seed_cells.get(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunStarted evidence",
                    cell.cell_id
                ))
            })?;
            let seed_artifact =
                committed_input_artifact(view, &cell.cell_id, &seed.seed_artifact.artifact_id)?;
            if seed_artifact.evidence.digest != seed.digest
                || seed_artifact.evidence.byte_len != seed.seed_artifact.byte_len
                || seed_artifact.evidence.media_type != seed.seed_artifact.media_type
                || seed_artifact.evidence.schema_id.as_ref() != Some(&seed.schema_id)
                || seed_artifact.evidence.semantic_type_id.as_ref() != Some(&seed.semantic_type_id)
                || seed_artifact.evidence.producer_node_id.is_some()
                || seed_artifact.evidence.producer_seed_id.as_ref() != Some(seed_id)
                || seed_artifact.evidence.artifact_role != events::ArtifactRole::SeedInput
            {
                return Err(RuntimeError::InputMaterialization(format!(
                    "seed cell {} committed artifact evidence does not match certified seed",
                    cell.cell_id
                )));
            }
            MaterializedCellTerminal::Seed {
                seed_id: seed_id.clone(),
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                content_digest: seed.digest.clone(),
            }
        }
        spec::CellProducer::Node(_) => {
            let projection = view
                .projections
                .cell_terminal(&cell.cell_id)
                .ok_or_else(|| {
                    RuntimeError::InputMaterialization(format!(
                        "input cell {} is not terminal",
                        cell.cell_id
                    ))
                })?;
            match projection {
                store::CellTerminalProjection::Produced {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    ..
                } => {
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "produced cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    let artifact = committed_input_artifact(view, &cell.cell_id, artifact_id)?;
                    if artifact.evidence.digest != *content_digest
                        || artifact.evidence.schema_id.as_ref() != Some(schema_id)
                        || artifact.evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
                        || artifact.evidence.producer_node_id.as_ref() != Some(node_id)
                        || artifact.evidence.producer_seed_id.is_some()
                        || artifact.evidence.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.attempt_id.as_ref() != Some(attempt_id)
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} committed artifact evidence does not match terminal projection",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Produced {
                        artifact_id: artifact_id.clone(),
                        content_digest: content_digest.clone(),
                    }
                }
                store::CellTerminalProjection::Skipped {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    skip_reason,
                    ..
                } => {
                    if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "input cell {} requires produced terminal but was skipped",
                            cell.cell_id
                        )));
                    }
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "skipped cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    MaterializedCellTerminal::Skipped {
                        skip_reason: skip_reason.clone(),
                    }
                }
            }
        }
    };
    Ok(MaterializedCell {
        cell_id: cell.cell_id.clone(),
        schema_id: cell.schema_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        value_lineage: cell.value_lineage.clone(),
        terminal,
    })
}

pub(crate) fn validate_seed_cells(
    runtime_spec: &CertifiedRuntimeSpec,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>> {
    let mut by_seed = BTreeMap::<_, _>::new();
    let mut by_cell = BTreeMap::<_, _>::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &runtime_spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing RunStarted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} metadata does not match certified seed",
                declared.seed_id
            )));
        }
        if let Some(required_digest) = &declared.required_digest {
            if &seed.digest != required_digest {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "RunStarted seed {} digest does not match certified required digest",
                    declared.seed_id
                )));
            }
        }
        if seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} artifact evidence does not match certified seed",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != runtime_spec.spec().seeds.len() {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted contains seed evidence not certified by the spec".to_owned(),
        ));
    }
    Ok(by_cell)
}

fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let mut available_cells = BTreeSet::<CellId>::new();
    let mut active_attempts = BTreeSet::<(NodeId, AttemptId)>::new();
    let mut side_effect_ledgers =
        BTreeMap::<events::SideEffectLedgerKey, HistoricalSideEffectLedger>::new();
    let mut seen_run_started = false;
    let mut produced_public_output = None::<events::PublicOutputCompletionEvidence>;
    let mut retention_manifest_projected_seq = None::<store::StreamSeq>;
    let mut completed = false;
    for (event_index, event) in stream.iter().enumerate() {
        if completed {
            return Err(RuntimeError::InvalidRunStream(
                "run stream contains events after RunCompleted".to_owned(),
            ));
        }
        if !seen_run_started
            && !matches!(event.payload(), events::KernelEventPayload::RunStarted(_))
        {
            return Err(RuntimeError::InvalidRunStream(
                "run stream events appeared before RunStarted".to_owned(),
            ));
        }
        match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => {
                if seen_run_started {
                    return Err(RuntimeError::InvalidRunStream(
                        "run stream contains multiple RunStarted events".to_owned(),
                    ));
                }
                seen_run_started = true;
                for cell_id in validate_seed_cells(runtime_spec, &payload.seed_cells)?.into_keys() {
                    available_cells.insert(cell_id);
                }
            }
            events::KernelEventPayload::RunCompleted(payload) => {
                if !active_attempts.is_empty() {
                    return Err(RuntimeError::InvalidRunStream(
                        "RunCompleted cannot finalize while state attempts are active".to_owned(),
                    ));
                }
                validate_historical_run_completed(
                    runtime_spec,
                    run_id,
                    event,
                    payload,
                    produced_public_output.as_ref(),
                    retention_manifest_projected_seq,
                )?;
                completed = true;
            }
            events::KernelEventPayload::ManualResolutionRecorded(payload) => {
                validate_historical_manual_resolution(
                    runtime_spec,
                    event_index,
                    event,
                    payload,
                    stream,
                )?;
            }
            events::KernelEventPayload::RetentionRefsAppended(_) => {}
            events::KernelEventPayload::RetentionManifestProjected(payload) => {
                if &payload.run_id == run_id && payload.spec_hash == *runtime_spec.spec_hash() {
                    retention_manifest_projected_seq = Some(event.seq());
                }
            }
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt started for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.state_kind != node.state_kind
                    || payload.state_version != node.state_version
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} carries state identity outside the certified spec",
                        payload.attempt_id, payload.node_id
                    )));
                }
                validate_attempt_start_boundary(runtime_spec, node, payload, &available_cells)?;
                if !active_attempts.insert((payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} was started more than once",
                        payload.attempt_id, payload.node_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} completed uncertified output cell {}",
                        payload.attempt_id, payload.node_id, payload.output_cell_id
                    )));
                }
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if runtime_spec
                    .node(&payload.node_id)
                    .expect("checked above")
                    .side_effect
                    .is_some()
                {
                    validate_historical_side_effect_failure(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt failure for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_historical_produced_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "fact recorded for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let caps = CertifiedRuntimeCapabilities::new(
                    node.node_id.clone(),
                    node.capability_bindings.clone(),
                );
                require_capability(
                    &caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_projected_attempt(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    "fact",
                )?;
                if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} was recorded outside an active started attempt",
                        payload.fact_key, payload.node_id, payload.attempt_id
                    )));
                }
                let fact = projections
                    .fact(&payload.node_id, &payload.attempt_id, &payload.fact_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "fact {} for node {} attempt {} is not projected",
                            payload.fact_key, payload.node_id, payload.attempt_id
                        ))
                    })?;
                if fact.event_id != *event.event_id()
                    || fact.request_schema_id != payload.request_schema_id
                    || fact.request_hash != payload.request_hash
                    || fact.response_schema_id != payload.response_schema_id
                    || fact.response_hash != payload.response_hash
                    || fact.artifact_id != payload.artifact_id
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} projection does not match authoritative event",
                        payload.fact_key
                    )));
                }
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if let Some(node_id) = &payload.node_id {
                    runtime_spec.node(node_id).ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "artifact referenced for uncertified node {node_id}"
                        ))
                    })?;
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output(runtime_spec, node, payload)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_produced(
                    runtime_spec,
                    projections,
                    event,
                    node,
                    payload,
                )?;
                produced_public_output = Some(events::PublicOutputCompletionEvidence {
                    public_output_schema_id: payload.public_schema_id.clone(),
                    public_output_event_id: event.event_id().clone(),
                });
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output failure by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_failed(projections, payload)?;
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_historical_side_effect_payload(
                    runtime_spec,
                    &active_attempts,
                    &mut side_effect_ledgers,
                    projections,
                    event.payload(),
                )?;
            }
        }
    }
    validate_atomic_terminal_pairs(runtime_spec, stream)?;
    validate_historical_bootstrap_run_batch(runtime_spec, run_id, stream)?;
    validate_historical_retention_ref_batches(runtime_spec, stream)?;
    validate_historical_retention_manifest_batches(runtime_spec, stream)?;
    validate_historical_terminal_tail(runtime_spec, run_id, stream)?;
    validate_atomic_side_effect_failure_pairs(runtime_spec, stream)?;
    validate_recovery_frontier(runtime_spec, projections)?;
    Ok(())
}

fn validate_historical_manual_resolution(
    runtime_spec: &CertifiedRuntimeSpec,
    event_index: usize,
    event: &store::KernelEventEnvelope,
    payload: &events::ManualResolutionRecorded,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    if event.run_id() != &payload.run_id || payload.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution event identity does not match certified run".to_owned(),
        ));
    }
    let manual = certified_manual_resolution_spec(&runtime_spec.spec().saga).ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "manual resolution was recorded without certified manual policy".to_owned(),
        )
    })?;
    if payload.evidence_schema_id != manual.evidence_schema {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution evidence schema does not match certified policy".to_owned(),
        ));
    }
    let authorization_schema_id = manual_authorization_proof_schema_id()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if payload.authorization_schema_id != authorization_schema_id {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution authorization schema does not match certified policy".to_owned(),
        ));
    }

    let prefix_projection = store::ProjectionSnapshot::rebuild_from_run_stream(
        stream.get(..event_index).ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "manual resolution prefix index was outside the run stream".to_owned(),
            )
        })?,
    )?;
    prefix_projection
        .require_manual_resolution_admissible(&payload.run_id, &runtime_spec.spec().saga)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if prefix_projection
        .derive_saga_projection(&payload.run_id, &runtime_spec.spec().saga)
        .manual_block_reason
        .is_none()
    {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution prefix lacks block reason".to_owned(),
        ));
    }
    Ok(())
}

fn certified_manual_resolution_spec(
    policy: &spec::SagaPolicySpec,
) -> Option<&spec::ManualResolutionEvidenceSpec> {
    match policy {
        spec::SagaPolicySpec::ManualResolution { manual } => Some(manual),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => Some(manual),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted { .. } => None,
    }
}

fn validate_attempt_start_boundary(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::StateAttemptStarted,
    available_cells: &BTreeSet<CellId>,
) -> Result<()> {
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        if !available_cells.contains(&cell_id) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} started before certified input cell {} was terminal",
                payload.attempt_id, node.node_id, cell_id
            )));
        }
    }
    Ok(())
}

fn validate_atomic_terminal_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                completions.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell skipped by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                ) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "public output produced by non-render node {}",
                        payload.node_id
                    )));
                }
                public_outputs.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (seq, node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            output_cell_id.clone(),
        )) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} completed without terminal cell {} in the same commit",
                attempt_id, node_id, output_cell_id
            )));
        }
    }
    for (seq, node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(*seq, node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "terminal cell {} for node {} lacks StateAttemptCompleted in the same commit",
                cell_id, node_id
            )));
        }
    }
    for (seq, node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            receipt_cell_id.clone(),
        );
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} was split from receipt terminal cell {}",
                node_id, attempt_id, receipt_cell_id
            )));
        }
    }
    Ok(())
}

fn validate_historical_retention_manifest_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        validate_historical_retention_manifest_batch(runtime_spec, &stream[..index], commit)?;
        index = end;
    }
    Ok(())
}

type RetentionRefKey = (ArtifactId, ContentDigest, events::ArtifactRole);

fn validate_historical_retention_ref_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        validate_historical_retention_ref_batch(runtime_spec, &stream[index..end])?;
        index = end;
    }
    Ok(())
}

pub(crate) fn validate_historical_bootstrap_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let Some(first) = stream.first() else {
        return Err(RuntimeError::InvalidRunStream(
            "run stream is missing sealed BootstrapRun genesis commit".to_owned(),
        ));
    };
    if first.seq() != store::StreamSeq::FIRST || first.ordinal() != store::CommitOrdinal::new(0) {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted must be the first event in the sealed BootstrapRun genesis commit"
                .to_owned(),
        ));
    }
    let first_seq = first.seq();
    let first_commit_key = first.commit_key().clone();
    let mut end = 1;
    while end < stream.len()
        && stream[end].seq() == first_seq
        && stream[end].commit_key() == &first_commit_key
    {
        end += 1;
    }
    validate_bootstrap_run_commit_payload_set(runtime_spec, run_id, &stream[..end])
}

fn validate_bootstrap_run_commit_payload_set(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let run_started = match commit.first().map(store::KernelEventEnvelope::payload) {
        Some(events::KernelEventPayload::RunStarted(payload)) => payload,
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit must start with RunStarted".to_owned(),
            ));
        }
    };
    validate_run_started_matches_certified_spec(runtime_spec, run_id, run_started)?;

    let config_count = runtime_spec.spec().config_refs.len();
    let expected_len = config_count + 6;
    if commit.len() != expected_len {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit has unexpected payload count".to_owned(),
        ));
    }

    let mut config_artifacts = Vec::with_capacity(config_count);
    for event in &commit[1..1 + config_count] {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit has non-config payload in config slot"
                    .to_owned(),
            ));
        };
        if payload.spec_hash != *runtime_spec.spec_hash()
            || payload.node_id.is_some()
            || payload.attempt_id.is_some()
            || payload.artifact_ref.role != events::ArtifactRole::TypedConfig
        {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit has invalid config artifact reference"
                    .to_owned(),
            ));
        }
        config_artifacts.push(store_artifact_from_event_ref(
            &payload.artifact_ref,
            None,
            None,
        ));
    }
    let config_artifacts = validate_config_artifacts(runtime_spec, config_artifacts)?;
    let expected_config_payloads =
        config_artifact_reference_payloads(runtime_spec.spec_hash(), &config_artifacts)?;
    for (event, expected) in commit[1..1 + config_count]
        .iter()
        .zip(expected_config_payloads)
    {
        if event.payload() != &expected {
            return Err(RuntimeError::InvalidRunStream(
                "sealed BootstrapRun genesis commit config references do not match certified config refs"
                    .to_owned(),
            ));
        }
    }

    let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
    let bootstrap_output_cell =
        runtime_spec
            .cell(&bootstrap_node.output_cell)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "bootstrap lifecycle node {} output cell {} is missing",
                    bootstrap_node.node_id, bootstrap_node.output_cell
                ))
            })?;
    let bootstrap_attempt_id =
        attempt_id(run_id, runtime_spec.spec_hash(), &bootstrap_node.node_id, 1)?;
    let genesis = GenesisContext {
        runtime_spec,
        run_id,
        node: bootstrap_node,
        output_cell: bootstrap_output_cell,
        attempt_id: &bootstrap_attempt_id,
        run_started,
        config_artifacts: &config_artifacts,
    };
    let (bootstrap_receipt_bytes, bootstrap_receipt_artifact) =
        bootstrap_run_receipt_artifact(&genesis)?;
    let expected_receipt_ref = event_artifact_ref_from_store(&bootstrap_receipt_artifact)?;
    let expected_refs = run_started_retention_refs(
        runtime_spec,
        run_started,
        &config_artifacts,
        &bootstrap_receipt_artifact,
    )?;

    let mut pos = 1 + config_count;
    let expected_start =
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            attempt_no: 1,
            state_kind: bootstrap_node.state_kind.clone(),
            state_version: bootstrap_node.state_version.clone(),
        });
    if commit[pos].payload() != &expected_start {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching StateAttemptStarted".to_owned(),
        ));
    }
    pos += 1;

    let expected_cell = events::KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: runtime_spec.spec_hash().clone(),
        node_id: bootstrap_node.node_id.clone(),
        cell_id: bootstrap_node.output_cell.clone(),
        scope_id: bootstrap_output_cell.scope_id.clone(),
        attempt_id: bootstrap_attempt_id.clone(),
        semantic_type_id: bootstrap_output_cell.semantic_type_id.clone(),
        schema_id: bootstrap_output_cell.schema_id.clone(),
        value_lineage: bootstrap_output_cell.value_lineage.clone(),
        artifact_id: bootstrap_receipt_artifact.artifact_id.clone(),
        content_digest: bootstrap_receipt_artifact.digest.clone(),
        producer_state_kind: Some(bootstrap_node.state_kind.clone()),
        producer_state_version: Some(bootstrap_node.state_version.clone()),
    });
    if commit[pos].payload() != &expected_cell {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching receipt cell".to_owned(),
        ));
    }
    pos += 1;

    let expected_completed =
        events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            output_cell_id: bootstrap_node.output_cell.clone(),
        });
    if commit[pos].payload() != &expected_completed {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching StateAttemptCompleted".to_owned(),
        ));
    }
    pos += 1;

    let expected_ref = events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
        spec_hash: runtime_spec.spec_hash().clone(),
        node_id: Some(bootstrap_node.node_id.clone()),
        attempt_id: Some(bootstrap_attempt_id.clone()),
        artifact_ref: expected_receipt_ref,
    });
    if commit[pos].payload() != &expected_ref {
        return Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching bootstrap receipt artifact reference"
                .to_owned(),
        ));
    }
    pos += 1;

    match commit[pos].payload() {
        events::KernelEventPayload::RetentionRefsAppended(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *runtime_spec.spec_hash()
                && payload.reason == events::RetentionReason::RunStarted
                && payload.refs == expected_refs =>
        {
            verify_artifact_bytes(
                bootstrap_receipt_bytes.as_bytes(),
                &bootstrap_receipt_artifact,
            )?;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(
            "sealed BootstrapRun genesis commit lacks matching run-start retention refs".to_owned(),
        )),
    }
}

fn validate_run_started_matches_certified_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_started: &events::RunStarted,
) -> Result<()> {
    if run_started.run_id != *run_id
        || run_started.spec_hash != *runtime_spec.spec_hash()
        || run_started.spec_artifact_id
            != ArtifactId::from_digest(
                runtime_spec.spec_hash().algorithm(),
                *runtime_spec.spec_hash().digest(),
            )
        || run_started.spec_media_type != runtime_spec.spec().media_type
        || run_started.spec_version != runtime_spec.spec().spec_version
        || run_started.lowering_version != runtime_spec.spec().lowering_version
        || run_started.public_output_schema_id
            != runtime_spec.spec().public_outputs.public_schema_id
        || run_started.canonicalizer_identity
            != runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
        || run_started.descriptor_identities != runtime_spec.spec().descriptor_identities
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted payload does not match the certified runtime spec".to_owned(),
        ));
    }
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let certificate_digest = certificate_canonical.content_digest();
    if run_started.certificate_artifact_id
        != ArtifactId::from_digest(certificate_digest.algorithm(), *certificate_digest.digest())
        || run_started.certificate_artifact_digest != certificate_digest
        || run_started.certificate_media_type
            != spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)?
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted certificate evidence does not match the certified runtime spec".to_owned(),
        ));
    }
    validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
    Ok(())
}

fn run_started_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_started: &events::RunStarted,
    config_artifacts: &[store::ArtifactEvidenceRef],
    bootstrap_receipt_artifact: &store::ArtifactEvidenceRef,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = Vec::with_capacity(3 + config_artifacts.len() + run_started.seed_cells.len());
    refs.push(events::RetentionRef {
        artifact_id: run_started.spec_artifact_id.clone(),
        content_digest: ContentDigest::from_digest(
            run_started.spec_hash.algorithm(),
            *run_started.spec_hash.digest(),
        ),
        role: events::ArtifactRole::TypedExecutionSpec,
    });
    refs.push(events::RetentionRef {
        artifact_id: run_started.certificate_artifact_id.clone(),
        content_digest: run_started.certificate_artifact_digest.clone(),
        role: events::ArtifactRole::TypedSpecCertificate,
    });
    refs.extend(config_artifacts.iter().map(retention_ref_for_artifact));
    let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
    refs.extend(seed_cells.values().map(|seed| events::RetentionRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        content_digest: seed.seed_artifact.content_digest.clone(),
        role: seed.seed_artifact.role,
    }));
    refs.push(retention_ref_for_artifact(bootstrap_receipt_artifact));
    Ok(refs)
}

fn validate_historical_retention_ref_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let retention_refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some((event, payload)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let run_started = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if run_started.len() > 1 {
        return Err(RuntimeError::InvalidRunStream(
            "commit contains multiple RunStarted payloads".to_owned(),
        ));
    }
    if run_started.len() == 1 {
        let run_started_refs = retention_refs
            .iter()
            .filter(|(_, payload)| payload.reason == events::RetentionReason::RunStarted)
            .count();
        if retention_refs.len() != 1 || run_started_refs != 1 {
            return Err(RuntimeError::InvalidRunStream(
                "RunStarted commit must include exactly one run-start retention refs append"
                    .to_owned(),
            ));
        }
    }
    if retention_refs.is_empty() {
        return Ok(());
    }

    let has_manifest_projection = commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    let artifact_refs = same_commit_artifact_reference_keys(commit);
    let typed_payload_refs = same_commit_typed_artifact_keys(commit);

    for (event, payload) in retention_refs {
        if event.run_id() != &payload.run_id {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention refs for run {} were appended to stream {}",
                payload.run_id,
                event.run_id()
            )));
        }
        if payload.spec_hash != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs spec hash does not match certified runtime spec".to_owned(),
            ));
        }
        if payload.refs.is_empty() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs append cannot be empty".to_owned(),
            ));
        }
        match payload.reason {
            events::RetentionReason::RunStarted => {
                if run_started.len() != 1 {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs must be appended in the RunStarted commit"
                            .to_owned(),
                    ));
                }
                let expected =
                    run_started_retention_ref_keys(runtime_spec, run_started[0], commit)?;
                let actual = payload
                    .refs
                    .iter()
                    .map(retention_ref_key)
                    .collect::<BTreeSet<_>>();
                if actual != expected {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs do not match launch artifact evidence".to_owned(),
                    ));
                }
            }
            events::RetentionReason::ManifestProjection => {
                if !has_manifest_projection {
                    return Err(RuntimeError::InvalidRunStream(
                        "manifest-projection retention refs must be appended with the manifest projection"
                            .to_owned(),
                    ));
                }
            }
            events::RetentionReason::RuntimeEvidence => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
            }
            events::RetentionReason::PublicOutput => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
                validate_public_output_retention_refs(runtime_spec, commit, payload)?;
            }
        }
    }
    Ok(())
}

fn validate_same_commit_retention_ref_evidence(
    payload: &events::RetentionRefsAppended,
    artifact_refs: &BTreeSet<RetentionRefKey>,
    typed_payload_refs: &BTreeSet<RetentionRefKey>,
) -> Result<()> {
    for retention_ref in &payload.refs {
        let key = retention_ref_key(retention_ref);
        if retention_ref_requires_artifact_reference(retention_ref.role)
            && !artifact_refs.contains(&key)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit artifact reference evidence",
                retention_ref.artifact_id
            )));
        }
        if !typed_payload_refs.contains(&key) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit typed payload evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
    payload: &events::RetentionRefsAppended,
) -> Result<()> {
    let public_outputs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if public_outputs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs must be appended with exactly one framework public-output payload"
                .to_owned(),
        ));
    }
    let public_output = public_outputs[0];
    let node = runtime_spec.node(&public_output.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public-output retention refs reference uncertified node {}",
            public_output.node_id
        ))
    })?;
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public-output retention refs were appended by non-render node {}",
            public_output.node_id
        )));
    }
    let allowed = public_output_retention_ref_keys(commit, public_output)?;
    for retention_ref in &payload.refs {
        if !allowed.contains(&retention_ref_key(retention_ref)) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public-output retention ref for artifact {} is not sealed to framework public-output evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn public_output_retention_ref_keys(
    commit: &[store::KernelEventEnvelope],
    public_output: &events::PublicOutputProduced,
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut allowed = BTreeSet::new();
    for event in commit {
        if let events::KernelEventPayload::CellProduced(payload) = event.payload() {
            if payload.node_id == public_output.node_id
                && payload.attempt_id == public_output.attempt_id
                && payload.cell_id == public_output.receipt_cell_id
            {
                allowed.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
        }
    }
    if let Some(artifact_id) = &public_output.rendered_artifact_id {
        allowed.insert((
            artifact_id.clone(),
            public_output.rendered_digest.clone(),
            events::ArtifactRole::PublicOutput,
        ));
    }
    if allowed.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs lack same-commit framework receipt/render evidence"
                .to_owned(),
        ));
    }
    Ok(allowed)
}

fn retention_ref_requires_artifact_reference(role: events::ArtifactRole) -> bool {
    staged_artifact_binding_kind(role).is_some()
}

fn retention_ref_key(retention_ref: &events::RetentionRef) -> RetentionRefKey {
    (
        retention_ref.artifact_id.clone(),
        retention_ref.content_digest.clone(),
        retention_ref.role,
    )
}

fn event_artifact_ref_key(artifact: &events::ArtifactEvidenceRef) -> RetentionRefKey {
    (
        artifact.artifact_id.clone(),
        artifact.content_digest.clone(),
        artifact.role,
    )
}

fn same_commit_artifact_reference_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                Some(event_artifact_ref_key(&payload.artifact_ref))
            }
            _ => None,
        })
        .collect()
}

fn same_commit_typed_artifact_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    let mut keys = BTreeSet::new();
    for event in commit {
        match event.payload() {
            events::KernelEventPayload::CellProduced(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.response_hash.clone(),
                    events::ArtifactRole::FactResponse,
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    keys.insert((
                        artifact_id.clone(),
                        payload.rendered_digest.clone(),
                        events::ArtifactRole::PublicOutput,
                    ));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                keys.insert((
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                if let (Some(artifact_id), Some(content_digest)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    keys.insert((
                        artifact_id.clone(),
                        content_digest.clone(),
                        events::ArtifactRole::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                keys.insert((
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    events::ArtifactRole::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                keys.insert((
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    events::ArtifactRole::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                keys.insert((
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    events::ArtifactRole::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                keys.insert((
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    events::ArtifactRole::Confirmation,
                ));
            }
            events::KernelEventPayload::ManualResolutionRecorded(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::ManualResolutionEvidence,
                ));
                keys.insert((
                    payload.authorization_artifact_id.clone(),
                    payload.authorization_hash.clone(),
                    events::ArtifactRole::ManualResolutionAuthorization,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            _ => {}
        }
    }
    keys
}

fn insert_error_diagnostic_key(error: &events::MfmErrorInfo, keys: &mut BTreeSet<RetentionRefKey>) {
    if let Some(diagnostic) = &error.diagnostic_ref {
        keys.insert(event_artifact_ref_key(diagnostic));
    }
}

fn run_started_retention_ref_keys(
    runtime_spec: &CertifiedRuntimeSpec,
    run_started: &events::RunStarted,
    commit: &[store::KernelEventEnvelope],
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut keys = BTreeSet::new();
    keys.insert((
        run_started.spec_artifact_id.clone(),
        ContentDigest::from_digest(
            run_started.spec_hash.algorithm(),
            *run_started.spec_hash.digest(),
        ),
        events::ArtifactRole::TypedExecutionSpec,
    ));
    keys.insert((
        run_started.certificate_artifact_id.clone(),
        run_started.certificate_artifact_digest.clone(),
        events::ArtifactRole::TypedSpecCertificate,
    ));
    for seed in &run_started.seed_cells {
        keys.insert(event_artifact_ref_key(&seed.seed_artifact));
    }
    for event in commit {
        if let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() {
            if payload.artifact_ref.role == events::ArtifactRole::TypedConfig {
                keys.insert(event_artifact_ref_key(&payload.artifact_ref));
            }
        }
    }
    let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
    for event in commit {
        if let events::KernelEventPayload::CellProduced(payload) = event.payload() {
            if payload.node_id == bootstrap_node.node_id
                && payload.cell_id == bootstrap_node.output_cell
            {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
        }
    }
    Ok(keys)
}

fn validate_historical_retention_manifest_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let projections = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if projections.is_empty() {
        return Ok(());
    }
    if projections.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains multiple manifest projections"
                .to_owned(),
        ));
    }
    let projection = projections[0];
    let retention_node = certified_retention_manifest_node(runtime_spec)?;
    if projection.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection spec hash does not match certified runtime spec"
                .to_owned(),
        ));
    }
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        &projection.run_id,
        pre_projection_stream,
        Some(retention_node.node_id.clone()),
    )?;
    if projection.manifest_seq != expected.manifest_seq
        || projection.manifest_digest != expected.evidence.digest
        || projection.previous_manifest_digest != expected.previous_manifest_digest
        || projection.manifest_artifact_id != expected.evidence.artifact_id
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection does not match the authoritative pre-projection stream"
                .to_owned(),
        ));
    }

    let receipt_bytes = retention_manifest_receipt_json(&expected, pre_projection_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection was not produced by exactly one framework retention receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != retention_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt cell does not match the projected manifest".to_owned(),
        ));
    }

    let started_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.spec_hash == *runtime_spec.spec_hash()
                        && payload.state_kind == retention_node.state_kind
                        && payload.state_version == retention_node.state_version
            )
        })
        .count();
    if started_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptStarted in the same commit"
                .to_owned(),
        ));
    }

    let completed_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.output_cell_id == retention_node.output_cell
            )
        })
        .count();
    if completed_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptCompleted in the same commit"
                .to_owned(),
        ));
    }

    let refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    let manifest_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::ManifestProjection)
        .collect::<Vec<_>>();
    if manifest_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must contain exactly one manifest retention refs append"
                .to_owned(),
        ));
    }
    let manifest_refs = manifest_refs[0];
    if manifest_refs.run_id != projection.run_id
        || manifest_refs.spec_hash != *runtime_spec.spec_hash()
        || manifest_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention refs are not sealed to manifest projection"
                .to_owned(),
        ));
    }
    let manifest_ref = &manifest_refs.refs[0];
    if manifest_ref.artifact_id != expected.evidence.artifact_id
        || manifest_ref.content_digest != expected.evidence.digest
        || manifest_ref.role != events::ArtifactRole::RetentionManifest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention ref does not match the manifest artifact"
                .to_owned(),
        ));
    }

    let receipt_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::RuntimeEvidence)
        .collect::<Vec<_>>();
    if receipt_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must retain its framework receipt artifact"
                .to_owned(),
        ));
    }
    let receipt_refs = receipt_refs[0];
    if receipt_refs.run_id != projection.run_id
        || receipt_refs.spec_hash != *runtime_spec.spec_hash()
        || receipt_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention refs are not sealed to runtime evidence"
                .to_owned(),
        ));
    }
    let receipt_ref = &receipt_refs.refs[0];
    if receipt_ref.artifact_id != receipt_artifact_id
        || receipt_ref.content_digest != receipt_digest
        || receipt_ref.role != events::ArtifactRole::StateOutput
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention ref does not match the receipt artifact"
                .to_owned(),
        ));
    }
    if refs.len() != 2 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains unsupported retention refs".to_owned(),
        ));
    }
    validate_retention_projection_commit_payload_set(
        commit,
        &retention_node.node_id,
        &produced.attempt_id,
        &retention_node.output_cell,
        &receipt_artifact_id,
        &receipt_digest,
        &expected.evidence.artifact_id,
    )?;
    Ok(())
}

fn validate_retention_projection_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    retention_node_id: &NodeId,
    attempt_id: &AttemptId,
    receipt_cell_id: &CellId,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
    manifest_artifact_id: &ArtifactId,
) -> Result<()> {
    let mut started = 0_usize;
    let mut produced = 0_usize;
    let mut completed = 0_usize;
    let mut artifact_referenced = 0_usize;
    let mut manifest_projected = 0_usize;
    let mut retention_refs = 0_usize;

    for event in commit {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == *retention_node_id && payload.attempt_id == *attempt_id =>
            {
                started += 1;
            }
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.cell_id == *receipt_cell_id
                    && payload.artifact_id == *receipt_artifact_id
                    && payload.content_digest == *receipt_digest =>
            {
                produced += 1;
            }
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.output_cell_id == *receipt_cell_id =>
            {
                completed += 1;
            }
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(retention_node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id)
                    && payload.artifact_ref.artifact_id == *receipt_artifact_id
                    && payload.artifact_ref.content_digest == *receipt_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
            {
                artifact_referenced += 1;
            }
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == *manifest_artifact_id =>
            {
                manifest_projected += 1;
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => match payload.reason {
                events::RetentionReason::ManifestProjection
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *manifest_artifact_id) =>
                {
                    retention_refs += 1;
                }
                events::RetentionReason::RuntimeEvidence
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *receipt_artifact_id) =>
                {
                    retention_refs += 1;
                }
                _ => {
                    return Err(RuntimeError::InvalidRunStream(
                        "retention manifest projection commit contains unsupported payload"
                            .to_owned(),
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::InvalidRunStream(
                    "retention manifest projection commit contains unsupported payload".to_owned(),
                ));
            }
        }
    }

    if started == 1
        && produced == 1
        && completed == 1
        && artifact_referenced == 1
        && manifest_projected == 1
        && retention_refs == 2
        && commit.len() == 7
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit does not match the sealed framework batch"
                .to_owned(),
        ))
    }
}

fn validate_historical_terminal_tail(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let resolve_node = certified_resolve_saga_terminal_node(runtime_spec)?;
    let mut last_retention_commit_end = None::<usize>;
    let mut terminal_commit = None::<(usize, usize, TerminalCommitKind)>;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        let has_retention_projection = commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        });
        if has_retention_projection {
            last_retention_commit_end = Some(end);
        }
        let has_completion_node_payload = commit
            .iter()
            .any(|event| payload_targets_node(event.payload(), &completion_node.node_id));
        let has_resolve_node_payload = commit
            .iter()
            .any(|event| payload_targets_node(event.payload(), &resolve_node.node_id));
        let run_completed_payloads = commit
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::RunCompleted(payload) => Some(payload),
                _ => None,
            })
            .collect::<Vec<_>>();
        let completion_count = run_completed_payloads.len();
        if has_completion_node_payload && has_resolve_node_payload {
            return Err(RuntimeError::InvalidRunStream(
                "terminal commit targets both CompleteRun and ResolveSagaTerminal".to_owned(),
            ));
        }
        if has_completion_node_payload || has_resolve_node_payload || completion_count > 0 {
            let kind = if has_resolve_node_payload
                || run_completed_payloads
                    .first()
                    .map(|payload| {
                        !matches!(payload.outcome, events::RunCompletionOutcome::Completed(_))
                    })
                    .unwrap_or(false)
            {
                TerminalCommitKind::ResolveSagaTerminal
            } else {
                TerminalCommitKind::CompleteRun
            };
            if terminal_commit.replace((index, end, kind)).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "run stream contains multiple terminal lifecycle commits".to_owned(),
                ));
            }
            if completion_count == 0 {
                return Err(RuntimeError::InvalidRunStream(
                    "terminal lifecycle commit is missing RunCompleted payload".to_owned(),
                ));
            }
            if completion_count != 1 {
                return Err(RuntimeError::InvalidRunStream(
                    "terminal lifecycle commit contains multiple RunCompleted payloads".to_owned(),
                ));
            }
        }
        index = end;
    }

    match (last_retention_commit_end, terminal_commit) {
        (Some(retention_end), Some((terminal_start, terminal_end, TerminalCommitKind::CompleteRun)))
            if retention_end == terminal_start =>
        {
            validate_historical_complete_run_batch(
                runtime_spec,
                run_id,
                &stream[..terminal_start],
                &stream[terminal_start..terminal_end],
            )
        }
        (Some(_), Some((_, _, TerminalCommitKind::CompleteRun))) => {
            Err(RuntimeError::InvalidRunStream(
                "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                    .to_owned(),
            ))
        }
        (retention, Some((terminal_start, terminal_end, TerminalCommitKind::ResolveSagaTerminal))) => {
            if retention.is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "sealed saga terminal appeared after retention manifest projection".to_owned(),
                ));
            }
            validate_historical_resolve_saga_terminal_batch(
                runtime_spec,
                run_id,
                &stream[..terminal_start],
                &stream[terminal_start..terminal_end],
            )
        }
        (Some(retention_end), None) if retention_end == stream.len() => Ok(()),
        (Some(_), None) => Err(RuntimeError::InvalidRunStream(
            "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                .to_owned(),
        )),
        (None, Some((_, _, TerminalCommitKind::CompleteRun))) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
        (None, None) => Ok(()),
    }
}

#[derive(Debug, Clone, Copy)]
enum TerminalCommitKind {
    CompleteRun,
    ResolveSagaTerminal,
}

fn payload_targets_node(payload: &events::KernelEventPayload, node_id: &NodeId) -> bool {
    match payload {
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellSkipped(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::FactRecorded(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            payload.node_id.as_ref() == Some(node_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.node_id == *node_id
        }
        payload => side_effect_payload_ref(payload)
            .map(|(payload_node_id, _, _, _)| payload_node_id == node_id)
            .unwrap_or(false),
    }
}

fn validate_historical_complete_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    pre_completion_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_payload = commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .expect("caller checked completion count");
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let pre_completion_projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(pre_completion_stream)?;
    let completion = run_completion_evidence(runtime_spec, &pre_completion_projection)?;
    let retention_manifest = projected_retention_manifest(run_id, &pre_completion_projection)?;
    if completion_payload.run_id != *run_id
        || completion_payload.spec_hash != *runtime_spec.spec_hash()
        || completion_payload.outcome
            != events::RunCompletionOutcome::Completed(Box::new(completion.clone()))
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted payload does not match sealed CompleteRun evidence".to_owned(),
        ));
    }

    let receipt_bytes =
        complete_run_receipt_json(&completion, retention_manifest, pre_completion_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == completion_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit was not produced by exactly one framework completion receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != completion_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun receipt cell does not match the sealed completion evidence".to_owned(),
        ));
    }
    let completion_cell = runtime_spec
        .cell(&completion_node.output_cell)
        .ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "completion lifecycle node {} output cell {} is missing",
                completion_node.node_id, completion_node.output_cell
            ))
        })?;
    let expected_receipt_ref = events::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        role: events::ArtifactRole::StateOutput,
        schema_id: completion_cell.schema_id.clone(),
        semantic_type_id: Some(completion_cell.semantic_type_id.clone()),
        content_digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
    };

    let completed_outcome = events::RunCompletionOutcome::Completed(Box::new(completion.clone()));
    validate_sealed_terminal_commit_batch(
        commit,
        SealedTerminalCommitValidation {
            label: "CompleteRun",
            run_id,
            spec_hash: runtime_spec.spec_hash(),
            outcome: &completed_outcome,
            node_id: &completion_node.node_id,
            attempt_id: &produced.attempt_id,
            receipt_cell_id: &completion_node.output_cell,
            receipt_artifact_id: &receipt_artifact_id,
            receipt_digest: &receipt_digest,
            expected_receipt_ref: &expected_receipt_ref,
        },
    )
}

fn validate_historical_resolve_saga_terminal_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    pre_resolution_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_payload = commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .expect("caller checked completion count");
    let resolve_node = certified_resolve_saga_terminal_node(runtime_spec)?;
    let pre_resolution_projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(pre_resolution_stream)?;
    let outcome =
        saga_terminal_completion_outcome(runtime_spec, run_id, &pre_resolution_projection)?;
    if completion_payload.run_id != *run_id
        || completion_payload.spec_hash != *runtime_spec.spec_hash()
        || completion_payload.outcome != outcome
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted payload does not match sealed ResolveSagaTerminal evidence".to_owned(),
        ));
    }

    let receipt_bytes =
        resolve_saga_terminal_receipt_json(runtime_spec, &outcome, pre_resolution_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == resolve_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "ResolveSagaTerminal commit was not produced by exactly one framework receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != resolve_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "ResolveSagaTerminal receipt cell does not match the sealed terminal evidence"
                .to_owned(),
        ));
    }
    let resolve_cell = runtime_spec
        .cell(&resolve_node.output_cell)
        .ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "resolve-saga-terminal lifecycle node {} output cell {} is missing",
                resolve_node.node_id, resolve_node.output_cell
            ))
        })?;
    let expected_receipt_ref = events::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        role: events::ArtifactRole::StateOutput,
        schema_id: resolve_cell.schema_id.clone(),
        semantic_type_id: Some(resolve_cell.semantic_type_id.clone()),
        content_digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
    };

    validate_sealed_terminal_commit_batch(
        commit,
        SealedTerminalCommitValidation {
            label: "ResolveSagaTerminal",
            run_id,
            spec_hash: runtime_spec.spec_hash(),
            outcome: &outcome,
            node_id: &resolve_node.node_id,
            attempt_id: &produced.attempt_id,
            receipt_cell_id: &resolve_node.output_cell,
            receipt_artifact_id: &receipt_artifact_id,
            receipt_digest: &receipt_digest,
            expected_receipt_ref: &expected_receipt_ref,
        },
    )
}

fn validate_sealed_terminal_commit_batch(
    commit: &[store::KernelEventEnvelope],
    expected: SealedTerminalCommitValidation<'_>,
) -> Result<()> {
    let SealedTerminalCommitValidation {
        label,
        run_id,
        spec_hash,
        outcome,
        node_id,
        attempt_id,
        receipt_cell_id,
        receipt_artifact_id,
        receipt_digest,
        expected_receipt_ref,
    } = expected;
    let mismatch = || {
        RuntimeError::InvalidRunStream(format!(
            "{label} commit does not match the sealed framework batch"
        ))
    };
    if commit.len() != 5 {
        return Err(mismatch());
    }
    match commit[0].payload() {
        events::KernelEventPayload::StateAttemptStarted(payload)
            if payload.node_id == *node_id && payload.attempt_id == *attempt_id => {}
        _ => return Err(mismatch()),
    }
    match commit[1].payload() {
        events::KernelEventPayload::CellProduced(payload)
            if payload.node_id == *node_id
                && payload.attempt_id == *attempt_id
                && payload.cell_id == *receipt_cell_id
                && payload.artifact_id == *receipt_artifact_id
                && payload.content_digest == *receipt_digest => {}
        _ => return Err(mismatch()),
    }
    match commit[2].payload() {
        events::KernelEventPayload::StateAttemptCompleted(payload)
            if payload.node_id == *node_id
                && payload.attempt_id == *attempt_id
                && payload.output_cell_id == *receipt_cell_id => {}
        _ => return Err(mismatch()),
    }
    match commit[3].payload() {
        events::KernelEventPayload::ArtifactReferenced(payload)
            if payload.node_id.as_ref() == Some(node_id)
                && payload.attempt_id.as_ref() == Some(attempt_id)
                && payload.artifact_ref == *expected_receipt_ref => {}
        _ => return Err(mismatch()),
    }
    match commit[4].payload() {
        events::KernelEventPayload::RunCompleted(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *spec_hash
                && payload.outcome == *outcome => {}
        _ => return Err(mismatch()),
    }
    Ok(())
}

pub(crate) fn require_projected_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_kind: &'static str,
) -> Result<store::AttemptStatus> {
    let attempt = projections.attempt(node_id, attempt_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "{event_kind} event references missing attempt {attempt_id} for node {node_id}"
        ))
    })?;
    Ok(attempt.status.clone())
}

fn validate_historical_produced_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellProduced,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("produced uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was produced by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_skipped_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellSkipped,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("skipped uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was skipped by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && skip_reason == &payload.skip_reason =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_public_output_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output",
    )? {
        store::AttemptStatus::Completed { output_cell_id }
            if output_cell_id == node.output_cell => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} is not backed by a completed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    if projections.cell_terminal(&node.output_cell).is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} has no terminal render receipt cell",
            payload.node_id
        )));
    }
    for cell in &payload.cells {
        let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "public output references uncertified cell {}",
                cell.cell_id
            ))
        })?;
        match projections.cell_terminal(&cell.cell_id) {
            Some(store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && certified.producer == cell.producer
                && certified.scope_id == cell.scope_id
                && certified.value_lineage == cell.value_lineage => {}
            _ => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "public output source cell {} is not backed by matching terminal evidence",
                    cell.cell_id
                )));
            }
        }
    }
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} is not backed by a render node",
            node.node_id
        )));
    };
    let expected_rendered_digest = public_output_rendered_digest(render, &payload.cells)?;
    if payload.rendered_digest != expected_rendered_digest {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} carries a rendered digest that does not match certified cells",
            node.node_id
        )));
    }
    let expected_receipt_digest = public_output_receipt_digest(
        render,
        &payload.cells,
        &expected_rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let expected_receipt_artifact_id = ArtifactId::from_digest(
        expected_receipt_digest.algorithm(),
        *expected_receipt_digest.digest(),
    );
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public output render node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let receipt_schema_id = spec::public_output_receipt_schema_id()?;
    match projections.cell_terminal(&node.output_cell) {
        Some(store::CellTerminalProjection::Produced {
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            ..
        }) if schema_id == &receipt_schema_id
            && semantic_type_id == &output_cell.semantic_type_id
            && artifact_id == &expected_receipt_artifact_id
            && content_digest == &expected_receipt_digest => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} has no matching terminal receipt cell",
                node.node_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        }) if event_id == event.event_id()
            && rendered_digest == &payload.rendered_digest
            && rendered_artifact_id == &payload.rendered_artifact_id =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output projection for schema {} does not match authoritative event",
            payload.public_schema_id
        ))),
    }
}

fn validate_historical_run_completed(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    event: &store::KernelEventEnvelope,
    payload: &events::RunCompleted,
    produced_public_output: Option<&events::PublicOutputCompletionEvidence>,
    retention_manifest_projected_seq: Option<store::StreamSeq>,
) -> Result<()> {
    if &payload.run_id != run_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references run {} while validating {}",
            payload.run_id, run_id
        )));
    }
    if payload.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted carries a spec hash outside the certified run".to_owned(),
        ));
    }
    let events::RunCompletionOutcome::Completed(completion) = &payload.outcome else {
        return Ok(());
    };
    if completion.public_output_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references public schema {} outside certified public outputs",
            completion.public_output_schema_id
        )));
    }
    match produced_public_output {
        Some(produced) if produced == completion.as_ref() => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted public-output evidence does not match preceding PublicOutputProduced"
                .to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before PublicOutputProduced".to_owned(),
        )),
    }?;
    match retention_manifest_projected_seq {
        Some(seq) if seq < event.seq() => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted must follow a prior retention manifest projection commit".to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
    }
}

fn validate_historical_public_output_failed(
    projections: &store::ProjectionSnapshot,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output failure",
    )? {
        store::AttemptStatus::Failed { .. } => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output failure for node {} attempt {} is not backed by a failed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced { .. })
        | Some(store::PublicOutputProjection::RenderFailed { .. }) => Ok(()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output failure for schema {} is not reflected in public-output projection",
            payload.public_schema_id
        ))),
    }
}

pub(crate) fn validate_spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != runtime_spec.spec().media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedExecutionSpec
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed execution spec artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

pub(crate) fn validate_certificate_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let media_type = spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedSpecCertificate
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed spec certificate artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

pub(crate) fn validate_config_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: Vec<store::ArtifactEvidenceRef>,
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut by_key = BTreeMap::new();
    for artifact in evidence {
        let Some(schema_id) = artifact.schema_id.clone() else {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence must carry schema_id".to_owned(),
            ));
        };
        if artifact.artifact_role != events::ArtifactRole::TypedConfig
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence has invalid role or producer metadata".to_owned(),
            ));
        }
        let key = format!("{}:{}", schema_id, artifact.digest);
        if by_key.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate typed config artifact evidence".to_owned(),
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config in &runtime_spec.spec().config_refs {
        let key = config_ref_key(config);
        let artifact = by_key.remove(&key).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing typed config artifact evidence for schema {} digest {}",
                config.schema_id, config.digest
            ))
        })?;
        if artifact.artifact_id != config.artifact_id
            || artifact.digest != config.digest
            || artifact.byte_len != config.byte_len
            || artifact.media_type != config.media_type
            || artifact.schema_id.as_ref() != Some(&config.schema_id)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "typed config artifact evidence for schema {} digest {} does not match certified config ref",
                config.schema_id, config.digest
            )));
        }
        validated.push(artifact);
    }
    if !by_key.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "typed config artifact evidence contains entries not certified by the spec".to_owned(),
        ));
    }
    Ok(validated)
}

fn config_artifacts_from_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<String, store::ArtifactEvidenceRef>> {
    let (start_seq, start_commit_key) = stream
        .iter()
        .find(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_)))
        .map(|event| (event.seq(), event.commit_key().clone()))
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            )
        })?;
    let mut referenced = Vec::new();
    for event in stream {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::TypedConfig {
            continue;
        }
        if payload.node_id.is_some() || payload.attempt_id.is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must not be scoped to a runner attempt"
                    .to_owned(),
            ));
        }
        if event.seq() != start_seq || event.commit_key() != &start_commit_key {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must be part of the RunStarted commit".to_owned(),
            ));
        }
        referenced.push(store_artifact_from_event_ref(
            &payload.artifact_ref,
            None,
            None,
        ));
    }
    let validated = validate_config_artifacts(runtime_spec, referenced)?;
    Ok(validated
        .into_iter()
        .map(|artifact| {
            let key = format!(
                "{}:{}",
                artifact
                    .schema_id
                    .as_ref()
                    .expect("validated config artifact has schema id"),
                artifact.digest
            );
            (key, artifact)
        })
        .collect())
}

fn artifact_refs_from_stream(
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<ArtifactId, CommittedArtifactReference>> {
    let mut artifacts = BTreeMap::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        for event in commit {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    for seed in &payload.seed_cells {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_seed_artifact(seed),
                                attempt_id: None,
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if staged_artifact_binding_kind(payload.artifact_ref.role).is_some() =>
                {
                    let (Some(node_id), Some(attempt_id)) =
                        (payload.node_id.clone(), payload.attempt_id.clone())
                    else {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} must be scoped to a producer attempt",
                            payload.artifact_ref.artifact_id
                        )));
                    };
                    if !artifact_reference_matches_same_commit_payload(commit, payload) {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} is not bound to a same-commit typed payload",
                            payload.artifact_ref.artifact_id
                        )));
                    }
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_artifact_from_event_ref(
                                    &payload.artifact_ref,
                                    Some(node_id),
                                    None,
                                ),
                                attempt_id: Some(attempt_id),
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role != events::ArtifactRole::TypedConfig =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "unsupported artifact reference role {} for {}",
                        artifact_role_name(payload.artifact_ref.role),
                        payload.artifact_ref.artifact_id
                    )));
                }
                _ => {}
            }
        }
        index = end;
    }
    Ok(artifacts)
}

fn artifact_reference_matches_same_commit_payload(
    commit: &[store::KernelEventEnvelope],
    reference: &events::ArtifactReferenced,
) -> bool {
    let (Some(reference_node_id), Some(reference_attempt_id)) =
        (&reference.node_id, &reference.attempt_id)
    else {
        return false;
    };
    commit.iter().any(|event| match event.payload() {
        events::KernelEventPayload::CellProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::StateOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.content_digest == reference.artifact_ref.content_digest
                && payload.schema_id == reference.artifact_ref.schema_id
                && reference.artifact_ref.semantic_type_id.as_ref()
                    == Some(&payload.semantic_type_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::FactResponse
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.response_hash == reference.artifact_ref.content_digest
                && payload.response_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.rendered_artifact_id.as_ref()
                    == Some(&reference.artifact_ref.artifact_id)
                && payload.rendered_digest == reference.artifact_ref.content_digest
                && payload.public_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        _ => false,
    })
}

fn event_artifact_refs_match(
    diagnostic: &events::ArtifactEvidenceRef,
    reference: &events::ArtifactReferenced,
) -> bool {
    diagnostic.artifact_id == reference.artifact_ref.artifact_id
        && diagnostic.role == reference.artifact_ref.role
        && diagnostic.schema_id == reference.artifact_ref.schema_id
        && diagnostic.semantic_type_id == reference.artifact_ref.semantic_type_id
        && diagnostic.content_digest == reference.artifact_ref.content_digest
        && diagnostic.byte_len == reference.artifact_ref.byte_len
        && diagnostic.media_type == reference.artifact_ref.media_type
}

fn insert_committed_artifact(
    artifacts: &mut BTreeMap<ArtifactId, CommittedArtifactReference>,
    artifact: CommittedArtifactReference,
) -> Result<()> {
    if let Some(existing) = artifacts.get(&artifact.evidence.artifact_id) {
        if existing != &artifact {
            return Err(RuntimeError::InvalidRunStream(format!(
                "conflicting committed artifact evidence for {}",
                artifact.evidence.artifact_id
            )));
        }
        return Ok(());
    }
    artifacts.insert(artifact.evidence.artifact_id.clone(), artifact);
    Ok(())
}

pub(crate) fn committed_config_artifact(
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<store::ArtifactEvidenceRef> {
    let key = config_ref_key(&node.config_ref);
    let artifact = view.config_artifacts.get(&key).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "node {} config artifact {} is not committed in the run stream",
            node.node_id, node.config_ref.artifact_id
        ))
    })?;
    if artifact.artifact_id != node.config_ref.artifact_id
        || artifact.digest != node.config_ref.digest
        || artifact.byte_len != node.config_ref.byte_len
        || artifact.media_type != node.config_ref.media_type
        || artifact.schema_id.as_ref() != Some(&node.config_ref.schema_id)
        || artifact.semantic_type_id.is_some()
        || artifact.producer_node_id.is_some()
        || artifact.producer_seed_id.is_some()
        || artifact.artifact_role != events::ArtifactRole::TypedConfig
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "node {} committed config artifact evidence does not match certified config ref",
            node.node_id
        )));
    }
    Ok(artifact.clone())
}

fn committed_input_artifact(
    view: &RuntimeRunView,
    cell_id: &CellId,
    artifact_id: &ArtifactId,
) -> Result<CommittedArtifactReference> {
    view.artifact_refs.get(artifact_id).cloned().ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {cell_id} artifact {artifact_id} is not committed in the run stream",
        ))
    })
}

fn store_artifact_from_event_ref(
    evidence: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<mfm_ids::SeedId>,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        digest: evidence.content_digest.clone(),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
        schema_id: Some(evidence.schema_id.clone()),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id,
        producer_seed_id,
        artifact_role: evidence.role,
    }
}

pub(crate) fn event_artifact_ref_from_store(
    artifact: &store::ArtifactEvidenceRef,
) -> Result<events::ArtifactEvidenceRef> {
    let schema_id = artifact.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "artifact {} cannot be referenced without schema id",
            artifact.artifact_id
        ))
    })?;
    Ok(events::ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id,
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    })
}

pub(crate) fn payload_spec_hash(payload: &events::KernelEventPayload) -> SpecHash {
    payload.spec_hash().clone()
}

pub(crate) fn store_seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        digest: seed.seed_artifact.content_digest.clone(),
        byte_len: seed.seed_artifact.byte_len,
        media_type: seed.seed_artifact.media_type.clone(),
        schema_id: Some(seed.seed_artifact.schema_id.clone()),
        semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: seed.seed_artifact.role,
    }
}
