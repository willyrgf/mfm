use super::*;

pub(super) fn validate_historical_terminal_tail(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    history: &RuntimeCommittedHistory,
    artifact_bytes: &store::ArtifactByteAuthorityMap,
) -> Result<()> {
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let resolve_node = certified_resolve_saga_terminal_node(runtime_spec)?;
    let mut last_retention_commit_end = None::<usize>;
    let mut terminal_commit = None::<(usize, usize, TerminalCommitKind)>;
    for batch in history.commits() {
        let commit = batch.events(stream);
        let has_retention_projection = commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        });
        if has_retention_projection {
            last_retention_commit_end = Some(batch.event_range.end);
        }
        let has_completion_node_payload = commit.iter().any(|event| {
            payload_targets_node_after_start(event.payload(), &completion_node.node_id)
        });
        let has_resolve_node_payload = commit
            .iter()
            .any(|event| payload_targets_node_after_start(event.payload(), &resolve_node.node_id));
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
            if terminal_commit
                .replace((batch.event_range.start, batch.event_range.end, kind))
                .is_some()
            {
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
    }

    match (last_retention_commit_end, terminal_commit) {
        (Some(retention_end), Some((terminal_start, terminal_end, TerminalCommitKind::CompleteRun)))
            if framework_start_gap_is_allowed(
                &stream[retention_end..terminal_start],
                &completion_node.node_id,
            ) =>
        {
            validate_historical_complete_run_batch(
                runtime_spec,
                run_id,
                &stream[..terminal_start],
                &stream[terminal_start..terminal_end],
                artifact_bytes,
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
                artifact_bytes,
            )
        }
        (Some(retention_end), None)
            if framework_start_gap_is_allowed(
                &stream[retention_end..],
                &completion_node.node_id,
            ) =>
        {
            Ok(())
        }
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
            .map(|(payload_node_id, _, _, _, _)| payload_node_id == node_id)
            .unwrap_or(false),
    }
}

fn payload_targets_node_after_start(
    payload: &events::KernelEventPayload,
    node_id: &NodeId,
) -> bool {
    !matches!(
        payload,
        events::KernelEventPayload::StateAttemptStarted(started) if started.node_id == *node_id
    ) && payload_targets_node(payload, node_id)
}

fn framework_start_gap_is_allowed(gap: &[store::KernelEventEnvelope], node_id: &NodeId) -> bool {
    gap.is_empty()
        || matches!(
            gap,
            [event]
                if matches!(
                    event.payload(),
                    events::KernelEventPayload::StateAttemptStarted(payload)
                        if payload.node_id == *node_id
                )
        )
}

fn validate_historical_complete_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    pre_completion_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
    artifact_bytes: &store::ArtifactByteAuthorityMap,
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
        store::ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            pre_completion_stream,
            artifact_bytes,
        )?;
    let completion = run_completion_evidence(runtime_spec, run_id, &pre_completion_projection)?;
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
    if !matches!(
        pre_completion_projection
            .attempt(&completion_node.node_id, &produced.attempt_id)
            .map(|attempt| &attempt.status),
        Some(store::AttemptStatus::Started { .. })
    ) {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit lacks matching started framework attempt in the prefix".to_owned(),
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
    let expected_receipt_store = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(completion_cell.schema_id.clone()),
        semantic_type_id: Some(completion_cell.semantic_type_id.clone()),
        producer_node_id: Some(completion_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let expected_receipt_ref = event_artifact_ref_from_store(&expected_receipt_store)?;

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
    artifact_bytes: &store::ArtifactByteAuthorityMap,
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
        store::ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            pre_resolution_stream,
            artifact_bytes,
        )?;
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
    if !matches!(
        pre_resolution_projection
            .attempt(&resolve_node.node_id, &produced.attempt_id)
            .map(|attempt| &attempt.status),
        Some(store::AttemptStatus::Started { .. })
    ) {
        return Err(RuntimeError::InvalidRunStream(
            "ResolveSagaTerminal commit lacks matching started framework attempt in the prefix"
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
    let expected_receipt_store = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(resolve_cell.schema_id.clone()),
        semantic_type_id: Some(resolve_cell.semantic_type_id.clone()),
        producer_node_id: Some(resolve_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let expected_receipt_ref = event_artifact_ref_from_store(&expected_receipt_store)?;

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
    if commit.len() != 4 {
        return Err(mismatch());
    }
    match commit[0].payload() {
        events::KernelEventPayload::CellProduced(payload)
            if payload.node_id == *node_id
                && payload.attempt_id == *attempt_id
                && payload.cell_id == *receipt_cell_id
                && payload.artifact_id == *receipt_artifact_id
                && payload.content_digest == *receipt_digest => {}
        _ => return Err(mismatch()),
    }
    match commit[1].payload() {
        events::KernelEventPayload::StateAttemptCompleted(payload)
            if payload.node_id == *node_id
                && payload.attempt_id == *attempt_id
                && payload.output_cell_id == *receipt_cell_id => {}
        _ => return Err(mismatch()),
    }
    match commit[2].payload() {
        events::KernelEventPayload::ArtifactReferenced(payload)
            if payload.node_id.as_ref() == Some(node_id)
                && payload.attempt_id.as_ref() == Some(attempt_id)
                && payload.artifact_ref == *expected_receipt_ref => {}
        _ => return Err(mismatch()),
    }
    match commit[3].payload() {
        events::KernelEventPayload::RunCompleted(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *spec_hash
                && payload.outcome == *outcome => {}
        _ => return Err(mismatch()),
    }
    Ok(())
}
