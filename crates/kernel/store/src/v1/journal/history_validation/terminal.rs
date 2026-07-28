use super::retention::{certified_framework_node, FrameworkRole};
use super::side_effects::side_effect_payload_ref;
use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalKind {
    Complete,
    Resolve,
}

#[derive(Debug)]
pub(super) struct TerminalHistoryFold {
    saw_retention: bool,
    completion_start_gap: Option<AttemptId>,
    terminal: Option<TerminalKind>,
    started_attempts: BTreeSet<(NodeId, AttemptId)>,
    last_sequence: Option<StreamSeq>,
}

impl TerminalHistoryFold {
    pub(super) fn new() -> Self {
        Self {
            saw_retention: false,
            completion_start_gap: None,
            terminal: None,
            started_attempts: BTreeSet::new(),
            last_sequence: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_suffix(
        &mut self,
        spec: &HistorySpec<'_>,
        run_id: &RunId,
        history: &JournalHistory<'_>,
        suffix_start: usize,
        projection: &ProjectionSnapshot,
        expected_saga_outcome: Option<&events::RunCompletionOutcome>,
    ) -> Result<()> {
        let complete_node = certified_framework_node(spec, FrameworkRole::Complete)?;
        let resolve_node = certified_framework_node(spec, FrameworkRole::Resolve)?;
        for batch in history.suffix_batches(suffix_start)? {
            let records = batch.records();
            let has_retention = records.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            });
            let has_complete = records.iter().any(|event| {
                payload_targets_node_after_start(event.payload(), &complete_node.node_id)
            });
            let has_resolve = records.iter().any(|event| {
                payload_targets_node_after_start(event.payload(), &resolve_node.node_id)
            });
            let completions = records
                .iter()
                .filter_map(|event| match event.payload() {
                    events::KernelEventPayload::RunCompleted(payload) => Some(payload),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if has_complete && has_resolve {
                return Err(invalid_history(
                    "terminal batch targets both CompleteRun and ResolveSagaTerminal",
                ));
            }
            if has_complete || has_resolve || !completions.is_empty() {
                if self.terminal.is_some() {
                    return Err(invalid_history(
                        "run journal contains multiple terminal lifecycle batches",
                    ));
                }
                if completions.len() != 1 {
                    return Err(invalid_history(
                        "terminal lifecycle batch must contain exactly one RunCompleted",
                    ));
                }
                let kind = if has_resolve
                    || !matches!(
                        completions[0].outcome,
                        events::RunCompletionOutcome::Completed(_)
                    ) {
                    TerminalKind::Resolve
                } else {
                    TerminalKind::Complete
                };
                match kind {
                    TerminalKind::Complete => {
                        if !self.saw_retention {
                            return Err(invalid_history(
                                "CompleteRun terminal appeared before retention projection",
                            ));
                        }
                        let expected_start =
                            records.iter().find_map(|event| match event.payload() {
                                events::KernelEventPayload::CellProduced(payload)
                                    if payload.node_id == complete_node.node_id =>
                                {
                                    Some(&payload.attempt_id)
                                }
                                _ => None,
                            });
                        if self.completion_start_gap.as_ref() != expected_start {
                            return Err(invalid_history(
                                "events appeared between retention and sealed CompleteRun batch",
                            ));
                        }
                        validate_complete_batch(
                            spec,
                            run_id,
                            history,
                            &batch,
                            projection,
                            complete_node,
                            &self.started_attempts,
                            self.last_sequence,
                        )?;
                    }
                    TerminalKind::Resolve => {
                        if self.saw_retention {
                            return Err(invalid_history(
                                "ResolveSagaTerminal appeared after retention projection",
                            ));
                        }
                        let expected = expected_saga_outcome.ok_or_else(|| {
                            invalid_history(
                                "ResolveSagaTerminal prefix has no certified terminal saga mode",
                            )
                        })?;
                        validate_resolve_batch(
                            spec,
                            run_id,
                            history,
                            &batch,
                            resolve_node,
                            expected,
                            &self.started_attempts,
                            self.last_sequence,
                        )?;
                    }
                }
                self.terminal = Some(kind);
                continue;
            }

            if self.terminal.is_some() {
                return Err(invalid_history(
                    "records appeared after terminal lifecycle batch",
                ));
            }
            if has_retention {
                self.saw_retention = true;
                self.completion_start_gap = None;
                self.apply_observations(records);
                continue;
            }
            if self.saw_retention {
                match records {
                    [event]
                        if matches!(
                            event.payload(),
                            events::KernelEventPayload::StateAttemptStarted(payload)
                                if payload.node_id == complete_node.node_id
                        ) =>
                    {
                        let events::KernelEventPayload::StateAttemptStarted(payload) =
                            event.payload()
                        else {
                            return Err(invalid_history(
                                "completion start gap does not contain StateAttemptStarted",
                            ));
                        };
                        if self
                            .completion_start_gap
                            .replace(payload.attempt_id.clone())
                            .is_some()
                        {
                            return Err(invalid_history(
                                "multiple completion starts appeared after retention",
                            ));
                        }
                    }
                    _ => {
                        return Err(invalid_history(
                            "records appeared after retention outside sealed CompleteRun tail",
                        ));
                    }
                }
            }
            self.apply_observations(records);
        }
        Ok(())
    }

    fn apply_observations(&mut self, records: &[KernelEventEnvelope]) {
        for event in records {
            self.last_sequence = Some(event.seq());
            if let events::KernelEventPayload::StateAttemptStarted(started) = event.payload() {
                self.started_attempts
                    .insert((started.node_id.clone(), started.attempt_id.clone()));
            }
        }
    }
}

fn payload_targets_node_after_start(
    payload: &events::KernelEventPayload,
    node_id: &NodeId,
) -> bool {
    !matches!(
        payload,
        events::KernelEventPayload::StateAttemptStarted(started)
            if started.node_id == *node_id
    ) && payload_targets_node(payload, node_id)
}

fn payload_targets_node(payload: &events::KernelEventPayload, node_id: &NodeId) -> bool {
    match payload {
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptInterrupted(payload) => payload.node_id == *node_id,
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
            .is_some_and(|(payload_node_id, ..)| payload_node_id == node_id),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_complete_batch(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    history: &JournalHistory<'_>,
    batch: &JournalBatch<'_>,
    projection: &ProjectionSnapshot,
    node: &spec::NodeSpec,
    started_attempts: &BTreeSet<(NodeId, AttemptId)>,
    prefix_sequence: Option<StreamSeq>,
) -> Result<()> {
    let completion = completion_evidence(spec, run_id, projection)?;
    let retention = projection
        .retention(run_id)
        .and_then(|retention| retention.manifest.as_ref())
        .ok_or_else(|| invalid_history("CompleteRun requires retention manifest evidence"))?;
    let outcome = events::RunCompletionOutcome::Completed(Box::new(completion.clone()));
    let recorded = batch
        .records()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| invalid_history("CompleteRun batch lacks RunCompleted"))?;
    if recorded.run_id != *run_id
        || recorded.spec_hash != *spec.spec_hash()
        || recorded.outcome != outcome
    {
        return Err(invalid_history(
            "RunCompleted does not match sealed CompleteRun evidence",
        ));
    }
    let receipt_bytes = complete_receipt_json(&completion, retention, prefix_sequence)?;
    validate_sealed_batch(
        spec,
        history,
        batch.records(),
        run_id,
        node,
        &outcome,
        receipt_bytes,
        started_attempts,
        "CompleteRun",
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_resolve_batch(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    history: &JournalHistory<'_>,
    batch: &JournalBatch<'_>,
    node: &spec::NodeSpec,
    expected_outcome: &events::RunCompletionOutcome,
    started_attempts: &BTreeSet<(NodeId, AttemptId)>,
    prefix_sequence: Option<StreamSeq>,
) -> Result<()> {
    let recorded = batch
        .records()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| invalid_history("ResolveSagaTerminal batch lacks RunCompleted"))?;
    if recorded.run_id != *run_id
        || recorded.spec_hash != *spec.spec_hash()
        || recorded.outcome != *expected_outcome
    {
        return Err(invalid_history(
            "RunCompleted does not match certified saga terminal frontier",
        ));
    }
    let receipt_bytes = resolve_receipt_json(spec, expected_outcome, prefix_sequence)?;
    validate_sealed_batch(
        spec,
        history,
        batch.records(),
        run_id,
        node,
        expected_outcome,
        receipt_bytes,
        started_attempts,
        "ResolveSagaTerminal",
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_sealed_batch(
    spec: &HistorySpec<'_>,
    history: &JournalHistory<'_>,
    records: &[KernelEventEnvelope],
    run_id: &RunId,
    node: &spec::NodeSpec,
    outcome: &events::RunCompletionOutcome,
    receipt_bytes: PlainCanonicalJsonBytes,
    started_attempts: &BTreeSet<(NodeId, AttemptId)>,
    label: &'static str,
) -> Result<()> {
    if records.len() != 4 {
        return Err(invalid_history(format!(
            "{label} batch does not match sealed four-record shape"
        )));
    }
    let digest = receipt_bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let events::KernelEventPayload::CellProduced(produced) = records[0].payload() else {
        return Err(invalid_history(format!("{label} batch lacks receipt cell")));
    };
    if produced.node_id != node.node_id
        || produced.cell_id != node.output_cell
        || produced.artifact_id != artifact_id
        || produced.content_digest != digest
        || !started_attempts.contains(&(node.node_id.clone(), produced.attempt_id.clone()))
    {
        return Err(invalid_history(format!(
            "{label} receipt cell does not match certified terminal evidence"
        )));
    }
    let cell = spec.cell(&node.output_cell).ok_or_else(|| {
        invalid_history(format!(
            "{label} output cell {} is absent from certified graph",
            node.output_cell
        ))
    })?;
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")
            .map_err(|error| invalid_history(error.to_string()))?,
        schema_id: Some(cell.schema_id.clone()),
        semantic_type_id: Some(cell.semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let expected_ref = event_artifact_ref(&evidence)?;
    let object = history
        .object(&artifact_id, &produced.evidence_hash)
        .ok_or_else(|| invalid_history(format!("{label} receipt object is absent")))?;
    if object.bytes.as_slice() != receipt_bytes.as_bytes() || object.evidence != evidence {
        return Err(invalid_history(format!(
            "{label} receipt object does not match canonical terminal receipt"
        )));
    }
    match records[1].payload() {
        events::KernelEventPayload::StateAttemptCompleted(payload)
            if payload.node_id == node.node_id
                && payload.attempt_id == produced.attempt_id
                && payload.output_cell_id == node.output_cell => {}
        _ => {
            return Err(invalid_history(format!(
                "{label} batch lacks matching attempt completion"
            )));
        }
    }
    match records[2].payload() {
        events::KernelEventPayload::ArtifactReferenced(payload)
            if payload.node_id.as_ref() == Some(&node.node_id)
                && payload.attempt_id.as_ref() == Some(&produced.attempt_id)
                && payload.artifact_ref == expected_ref => {}
        _ => {
            return Err(invalid_history(format!(
                "{label} batch lacks matching receipt reference"
            )));
        }
    }
    match records[3].payload() {
        events::KernelEventPayload::RunCompleted(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *spec.spec_hash()
                && payload.outcome == *outcome => {}
        _ => {
            return Err(invalid_history(format!(
                "{label} batch lacks matching RunCompleted"
            )));
        }
    }
    Ok(())
}

fn completion_evidence(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    projection: &ProjectionSnapshot,
) -> Result<events::PublicOutputCompletionEvidence> {
    let schema_id = spec.spec().public_outputs.public_schema_id.clone();
    match projection.public_output(run_id, &schema_id) {
        Some(PublicOutputProjection::Produced { event_id, .. }) => {
            Ok(events::PublicOutputCompletionEvidence {
                public_output_schema_id: schema_id,
                public_output_event_id: event_id.clone(),
            })
        }
        _ => Err(invalid_history(
            "CompleteRun requires produced public-output evidence",
        )),
    }
}

fn complete_receipt_json(
    completion: &events::PublicOutputCompletionEvidence,
    retention: &RetentionManifestProjection,
    prefix_seq: Option<StreamSeq>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_event_id": completion.public_output_event_id.as_str(),
        "public_output_schema_id": completion.public_output_schema_id.as_str(),
        "retention_manifest_artifact_id": retention.manifest_artifact_id.as_str(),
        "retention_manifest_digest": retention.manifest_digest.as_str(),
        "retention_manifest_seq": retention.manifest_seq,
        "pre_completion_stream_seq": prefix_seq.map(StreamSeq::as_u64),
    }))
}

fn resolve_receipt_json(
    spec: &HistorySpec<'_>,
    outcome: &events::RunCompletionOutcome,
    prefix_seq: Option<StreamSeq>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_schema_id": spec.spec().public_outputs.public_schema_id.as_str(),
        "terminal_outcome": outcome.kind(),
        "pre_resolution_stream_seq": prefix_seq.map(StreamSeq::as_u64),
    }))
}

fn event_artifact_ref(evidence: &ArtifactEvidenceRef) -> Result<events::ArtifactEvidenceRef> {
    let schema_id = evidence
        .schema_id
        .clone()
        .ok_or_else(|| invalid_history("terminal receipt evidence lacks schema id"))?;
    Ok(events::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        role: evidence.artifact_role,
        schema_id,
        semantic_type_id: evidence.semantic_type_id.clone(),
        content_digest: evidence.digest.clone(),
        evidence_hash: evidence.evidence_hash()?,
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
    })
}
