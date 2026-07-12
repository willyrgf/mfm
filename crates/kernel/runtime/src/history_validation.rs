use super::*;

mod history_validation_retention;
#[path = "history_validation_terminal.rs"]
mod history_validation_terminal;

use self::history_validation_retention::{
    validate_historical_retention_manifest_batches, validate_historical_retention_ref_batches,
};
use self::history_validation_terminal::validate_historical_terminal_tail;

pub(super) fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
    history: &RuntimeCommittedHistory,
    artifact_bytes: &store::ArtifactByteAuthorityMap,
) -> Result<()> {
    let mut available_cells = BTreeSet::<CellId>::new();
    let mut active_attempts = BTreeSet::<(NodeId, AttemptId)>::new();
    let mut side_effect_ledgers = BTreeMap::<SideEffectPairId, HistoricalSideEffectLedger>::new();
    let mut seen_run_admitted = false;
    let mut produced_public_output = None::<events::PublicOutputCompletionEvidence>;
    let mut retention_manifest_projected_seq = None::<store::StreamSeq>;
    let mut completed = false;
    for (event_index, event) in stream.iter().enumerate() {
        if completed {
            return Err(RuntimeError::InvalidRunStream(
                "run stream contains events after RunCompleted".to_owned(),
            ));
        }
        if !seen_run_admitted
            && !matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))
        {
            return Err(RuntimeError::InvalidRunStream(
                "run stream events appeared before RunAdmitted".to_owned(),
            ));
        }
        match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => {
                if seen_run_admitted {
                    return Err(RuntimeError::InvalidRunStream(
                        "run stream contains multiple RunAdmitted events".to_owned(),
                    ));
                }
                seen_run_admitted = true;
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
                    artifact_bytes,
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
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        runtime_spec,
                        &side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        stream_output_cell_is_skipped(stream, node, &payload.attempt_id),
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptInterrupted(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt interrupted for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt interruption for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_failure(
                        runtime_spec,
                        &side_effect_ledgers,
                        node,
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
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        runtime_spec,
                        &side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        false,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node_uses_side_effect_terminal_validation(node) {
                    validate_historical_side_effect_terminal(
                        runtime_spec,
                        &side_effect_ledgers,
                        node,
                        &payload.attempt_id,
                        true,
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
                if !node.fact_descriptor_allowlist.iter().any(|reference| {
                    &reference.descriptor_hash == payload.claim.fact_descriptor_hash()
                }) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact descriptor {} is not certified for producing node {}",
                        payload.claim.fact_descriptor_hash(),
                        payload.node_id
                    )));
                }
                let caps = CertifiedRuntimeCapabilities::for_node(node);
                let producer = payload.claim.producer();
                require_capability(
                    &caps,
                    producer.capability_kind(),
                    producer.capability_version(),
                    &node.node_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_adapter(node, producer.adapter_kind(), producer.adapter_version())
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_projected_attempt(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    "fact",
                )?;
                if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    let fact_key = payload.claim.subject().fact_key();
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} was recorded outside an active started attempt",
                        fact_key, payload.node_id, payload.attempt_id
                    )));
                }
                let claim_id = mfm_facts::derive_fact_claim_id(
                    event.run_id().clone(),
                    event.seq().as_u64(),
                    event.ordinal().as_u32(),
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                let fact_key = payload.claim.subject().fact_key();
                let record = projections.fact_record(&claim_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} is not projected",
                        fact_key, payload.node_id, payload.attempt_id
                    ))
                })?;
                if record.source_event_id != *event.event_id()
                    || record.node_id != payload.node_id
                    || record.attempt_id != payload.attempt_id
                    || record.claim != payload.claim
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} projection does not match authoritative event",
                        fact_key
                    )));
                }
                match payload.claim.visibility() {
                    mfm_facts::FactVisibility::Indexed { .. } => {
                        let index = projections.fact_index_entry(&claim_id).ok_or_else(|| {
                            RuntimeError::InvalidRunStream(format!(
                                "indexed fact {} for node {} attempt {} is not projected",
                                fact_key, payload.node_id, payload.attempt_id
                            ))
                        })?;
                        if index.source_event_id != *event.event_id()
                            || index.fact_key != *fact_key
                            || index.fact_descriptor_hash != *payload.claim.fact_descriptor_hash()
                            || index.response_hash != *payload.claim.response().response_hash()
                            || index.artifact_id != *payload.claim.response().artifact_id()
                        {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "indexed fact {} projection does not match authoritative event",
                                fact_key
                            )));
                        }
                    }
                    mfm_facts::FactVisibility::RunPrivate => {
                        if projections.fact_index_entry(&claim_id).is_some() {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "private fact {} has an index projection",
                                fact_key
                            )));
                        }
                    }
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
                validate_historical_public_output_failed(projections, event.run_id(), payload)?;
            }
            events::KernelEventPayload::ResourceLaneClaimIntent(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                return Err(RuntimeError::InvalidRunStream(
                    "run stream contains unmaterialized resource-lane intent".to_owned(),
                ));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::ResourceLaneClaimed(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_) => {
                validate_historical_side_effect_payload(
                    runtime_spec,
                    event.run_id(),
                    &active_attempts,
                    &mut side_effect_ledgers,
                    projections,
                    event.payload(),
                )?;
            }
        }
    }
    validate_atomic_terminal_pairs(runtime_spec, stream)?;
    validate_historical_run_admission_batch(runtime_spec, run_id, stream, history)?;
    validate_historical_retention_ref_batches(runtime_spec, stream, history)?;
    validate_historical_retention_manifest_batches(runtime_spec, stream, history, artifact_bytes)?;
    validate_historical_terminal_tail(runtime_spec, run_id, stream, history, artifact_bytes)?;
    validate_atomic_side_effect_failure_pairs(runtime_spec, stream)?;
    AttemptRecoveryLifecycle::validate_frontier(runtime_spec, run_id, projections)?;
    Ok(())
}

fn validate_historical_manual_resolution(
    runtime_spec: &CertifiedRuntimeSpec,
    event_index: usize,
    event: &store::KernelEventEnvelope,
    payload: &events::ManualResolutionRecorded,
    stream: &[store::KernelEventEnvelope],
    artifact_bytes: &store::ArtifactByteAuthorityMap,
) -> Result<()> {
    if event.run_id() != &payload.run_id || payload.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution event identity does not match certified run".to_owned(),
        ));
    }
    let manual = certified_manual_resolution_spec(&runtime_spec.spec().saga)?;
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

    let prefix_projection = store::ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
        stream.get(..event_index).ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "manual resolution prefix index was outside the run stream".to_owned(),
            )
        })?,
        artifact_bytes,
    )?;
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    prefix_projection
        .require_manual_resolution_admissible(
            &payload.run_id,
            &runtime_spec.spec().saga,
            &terminal_policies,
        )
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    let saga = prefix_projection.derive_saga_projection(
        &payload.run_id,
        &runtime_spec.spec().saga,
        &terminal_policies,
    )?;
    if saga.manual_block_reason.is_none() {
        return Err(RuntimeError::InvalidRunStream(
            "manual resolution prefix lacks block reason".to_owned(),
        ));
    }
    Ok(())
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

fn stream_output_cell_is_skipped(
    stream: &[store::KernelEventEnvelope],
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> bool {
    stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::CellSkipped(payload)
                if payload.node_id == node.node_id
                    && payload.attempt_id == *attempt_id
                    && payload.cell_id == node.output_cell
        )
    })
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

fn validate_historical_run_admission_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    history: &RuntimeCommittedHistory,
) -> Result<()> {
    let Some(first) = stream.first() else {
        return Err(RuntimeError::InvalidRunStream(
            "run stream is missing RunAdmitted root event".to_owned(),
        ));
    };
    if first.seq() != store::StreamSeq::FIRST || first.ordinal() != store::CommitOrdinal::new(0) {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted must be the first event in the run stream".to_owned(),
        ));
    }
    let first_commit = history
        .commits()
        .first()
        .map(|batch| batch.events(stream))
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run stream is missing RunAdmitted root event".to_owned(),
            )
        })?;
    if first_commit.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted commit must contain exactly one root event".to_owned(),
        ));
    }
    let events::KernelEventPayload::RunAdmitted(run_admitted) = first.payload() else {
        return Err(RuntimeError::InvalidRunStream(
            "run stream must start with RunAdmitted".to_owned(),
        ));
    };
    validate_run_admitted_matches_certified_spec(runtime_spec, run_id, run_admitted)
}

fn validate_run_admitted_matches_certified_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_admitted: &events::RunAdmitted,
) -> Result<()> {
    validate_run_identity_material(runtime_spec, run_id, run_admitted)?;
    if run_admitted.run_id != *run_id
        || run_admitted.spec_hash != *runtime_spec.spec_hash()
        || run_admitted.spec_version != runtime_spec.spec().spec_version
        || run_admitted.lowering_version != runtime_spec.spec().lowering_version
        || run_admitted.public_output_schema_id
            != runtime_spec.spec().public_outputs.public_schema_id
        || run_admitted.saga_policy_digest != runtime_spec.spec().saga.saga_policy_digest()?
        || run_admitted.canonicalizer_identity
            != runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
        || run_admitted.descriptor_identities != runtime_spec.spec().descriptor_identities
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted payload does not match the certified runtime spec".to_owned(),
        ));
    }
    validate_spec_artifact(
        runtime_spec,
        store_artifact_from_run_ref(&run_admitted.spec_artifact),
    )?;
    validate_certificate_artifact(
        runtime_spec,
        store_artifact_from_run_ref(&run_admitted.certificate_artifact),
    )?;
    validate_config_artifacts(
        runtime_spec,
        run_admitted
            .config_artifacts
            .iter()
            .map(store_artifact_from_run_ref)
            .collect(),
    )?;
    validate_fact_descriptor_artifact_refs(runtime_spec, &run_admitted.fact_descriptor_artifacts)?;
    validate_seed_cells(runtime_spec, &run_admitted.seed_cells)?;
    Ok(())
}

pub(super) fn validate_run_identity_material(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_admitted: &events::RunAdmitted,
) -> Result<()> {
    if &run_admitted.run_id != run_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "run stream contains RunAdmitted for {} while executing {}",
            run_admitted.run_id, run_id
        )));
    }
    if &run_admitted.spec_hash != runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunAdmitted spec hash {} does not match certified {}",
            run_admitted.spec_hash,
            runtime_spec.spec_hash()
        )));
    }
    if run_admitted.identity_material.certified_spec_hash != run_admitted.spec_hash {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted identity material spec hash does not match event spec hash".to_owned(),
        ));
    }
    let derived_run_id = run_admitted.identity_material.derive_run_id()?;
    if derived_run_id != run_admitted.run_id {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted run id does not match identity material".to_owned(),
        ));
    }
    Ok(())
}

fn require_projected_attempt(
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
        || cell.context != payload.context
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal_for_run(event.run_id(), &payload.cell_id) {
        Some(store::CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            evidence_hash,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest
            && evidence_hash == &payload.evidence_hash =>
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
        || cell.context != payload.context
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal_for_run(event.run_id(), &payload.cell_id) {
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
    if projections
        .cell_terminal_for_run(event.run_id(), &node.output_cell)
        .is_none()
    {
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
        match projections.cell_terminal_for_run(event.run_id(), &cell.cell_id) {
            Some(store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                evidence_hash,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && evidence_hash == &cell.evidence_hash
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
    match projections.cell_terminal_for_run(event.run_id(), &node.output_cell) {
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
    match projections.public_output(event.run_id(), &payload.public_schema_id) {
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
    run_id: &RunId,
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
    match projections.public_output(run_id, &payload.public_schema_id) {
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
        || evidence.schema_id.as_ref()
            != Some(
                &spec::typed_execution_spec_schema_id()
                    .map_err(|error| RuntimeError::Identity(error.to_string()))?,
            )
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
        || evidence.schema_id.as_ref()
            != Some(
                &mfm_certify::typed_spec_certificate_schema_id()
                    .map_err(|error| RuntimeError::Identity(error.to_string()))?,
            )
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

fn validate_fact_descriptor_artifact_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    artifacts: &[events::RunArtifactEvidenceRef],
) -> Result<()> {
    let required = runtime_spec.fact_descriptor_hashes();
    let schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    let media_type = spec::MediaType::new("application/json")?;
    let mut admitted = BTreeSet::new();

    for artifact in artifacts {
        if artifact.role != events::ArtifactRole::FactDescriptor
            || artifact.content_digest.algorithm() != mfm_ids::DigestAlgorithm::Sha256JcsV1
            || artifact.media_type != media_type
            || artifact.schema_id.as_ref() != Some(&schema_id)
            || artifact.semantic_type_id.is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact descriptor artifact {} has invalid metadata",
                artifact.artifact_id
            )));
        }
        let expected_artifact_id = ArtifactId::from_digest(
            artifact.content_digest.algorithm(),
            *artifact.content_digest.digest(),
        );
        if artifact.artifact_id != expected_artifact_id {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact descriptor artifact {} does not match descriptor digest {}",
                artifact.artifact_id, artifact.content_digest
            )));
        }
        if !admitted.insert(artifact.content_digest.clone()) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate fact descriptor artifact {}",
                artifact.content_digest
            )));
        }
    }

    if admitted != required {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted fact descriptor artifacts do not match certified descriptor allow-lists"
                .to_owned(),
        ));
    }
    Ok(())
}
