use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{StagedArtifactBindingKind, StagedSideEffectArtifactPhase};
use crate::{
    require_adapter, require_attempt, require_capability, CertifiedRuntimeCapabilities,
    CertifiedRuntimeSpec, Result, RuntimeError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoricalSideEffectPhase {
    IntentPersisted,
    Claimed,
    InvocationPrepared,
    InvocationStarted,
    NotSubmittedProven,
    SubmissionObserved,
    SubmissionUnknown,
    ReceiptObserved,
    ConfirmationObserved,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoricalSideEffectLedger {
    node_id: NodeId,
    attempt_id: AttemptId,
    phase: HistoricalSideEffectPhase,
}

pub(crate) fn validate_historical_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let (node_id, attempt_id, ledger_key, phase) = side_effect_payload_ref(payload)
        .ok_or_else(|| RuntimeError::InvalidRunStream("expected side-effect payload".to_owned()))?;
    let node = runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("side-effect event for uncertified node {node_id}"))
    })?;
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "non-side-effect node {} emitted side-effect ledger event",
            node.node_id
        )));
    }
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect ledger {} for node {} attempt {} was recorded outside an active started attempt",
            ledger_key, node_id, attempt_id
        )));
    }
    validate_side_effect_ledger_purpose(runtime_spec, projections, node, payload)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;

    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect intent for node {} carries uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
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
            if ledgers
                .insert(
                    ledger_key.clone(),
                    HistoricalSideEffectLedger {
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        phase,
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} persisted intent more than once",
                    ledger_key
                )));
            }
        }
        _ => {
            let ledger = ledgers.get_mut(ledger_key).ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} advanced before intent was persisted",
                    ledger_key
                ))
            })?;
            if ledger.node_id != *node_id || ledger.attempt_id != *attempt_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed node or attempt authority",
                    ledger_key
                )));
            }
            ledger.phase = phase;
        }
    }

    Ok(())
}

pub(crate) fn side_effect_payload_ref(
    payload: &events::KernelEventPayload,
) -> Option<(
    &NodeId,
    &AttemptId,
    &events::SideEffectLedgerKey,
    HistoricalSideEffectPhase,
)> {
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::IntentPersisted,
        )),
        events::KernelEventPayload::SideEffectClaimed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationPrepared,
        )),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationStarted,
        )),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::NotSubmittedProven,
        )),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionObserved,
        )),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionUnknown,
        )),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ReceiptObserved,
        )),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ConfirmationObserved,
        )),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Ambiguous,
        )),
        events::KernelEventPayload::SideEffectFailed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Failed,
        )),
        _ => None,
    }
}

fn side_effect_payload_ledger_purpose(
    payload: &events::KernelEventPayload,
) -> Option<&events::SideEffectLedgerPurpose> {
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => Some(&payload.ledger_purpose),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            Some(&payload.ledger_purpose)
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => Some(&payload.ledger_purpose),
        events::KernelEventPayload::SideEffectFailed(payload) => Some(&payload.ledger_purpose),
        _ => None,
    }
}

fn validate_side_effect_ledger_purpose(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let purpose = side_effect_payload_ledger_purpose(payload).ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
    })?;
    let Some(forward_node_id) = runtime_spec.forward_node_for_remediation(&node.node_id) else {
        if matches!(purpose, events::SideEffectLedgerPurpose::Forward) {
            return Ok(());
        }
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "forward side-effect node {} emitted remediation ledger purpose",
            node.node_id
        )));
    };

    let events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } = purpose else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "remediation node {} emitted forward ledger purpose",
            node.node_id
        )));
    };
    let forward = projections.side_effect(forward_ledger_key).ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "remediation node {} linked missing forward ledger {}",
            node.node_id, forward_ledger_key
        ))
    })?;
    if matches!(
        &forward.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    ) && forward.intent.node_id == *forward_node_id
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "remediation node {} linked ledger {} outside certified forward node {}",
            node.node_id, forward_ledger_key, forward_node_id
        )))
    }
}

pub(crate) fn validate_historical_side_effect_confirmation(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node_id, attempt_id
        )))
    }
}

pub(crate) fn validate_historical_side_effect_failure(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::Failed {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} failed without side-effect failure evidence",
            node_id, attempt_id
        )))
    }
}

fn historical_side_effect_ledger_for_attempt<'a>(
    ledgers: &'a BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<&'a HistoricalSideEffectLedger> {
    let mut found = None;
    for ledger in ledgers.values() {
        if ledger.node_id == *node_id
            && ledger.attempt_id == *attempt_id
            && found.replace(ledger).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledgers in history",
                node_id, attempt_id
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} lacks ledger evidence",
            node_id, attempt_id
        ))
    })
}

pub(crate) fn validate_atomic_side_effect_failure_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::SideEffectFailed(payload) => {
                side_effect_failures.insert(
                    (
                        event.seq(),
                        payload.node_id.clone(),
                        payload.attempt_id.clone(),
                    ),
                    payload.retryable,
                );
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node.side_effect.is_some() {
                    attempt_failures.insert(
                        (
                            event.seq(),
                            payload.node_id.clone(),
                            payload.attempt_id.clone(),
                        ),
                        payload.retryable,
                    );
                }
            }
            _ => {}
        }
    }
    for (failure, side_effect_retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
            Some(_) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} disagrees with StateAttemptFailed retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} lacks StateAttemptFailed in the same commit",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect attempt failure for node {} attempt {} lacks SideEffectFailed in the same commit",
                failure.1, failure.2
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_recovery_frontier(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if let Some(terminal) = projections.cell_terminal(&node.output_cell) {
            let attempt_id = validate_terminal_cell_has_completed_attempt(
                runtime_spec,
                projections,
                node,
                terminal,
            )?;
            if node.side_effect.is_some() {
                validate_side_effect_terminal_evidence(projections, node, &attempt_id)?;
            }
        }
        let mut started = None;
        for ((attempt_node_id, attempt_id), projection) in projections.attempts() {
            if attempt_node_id != &node.node_id {
                continue;
            }
            match &projection.status {
                store::AttemptStatus::Started { .. } => {
                    if projections.cell_terminal(&node.output_cell).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has a started attempt after its output cell became terminal",
                            node.node_id
                        )));
                    }
                    if started.replace(attempt_id.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has multiple started attempts during recovery",
                            node.node_id
                        )));
                    }
                }
                store::AttemptStatus::Completed { output_cell_id }
                    if projections.cell_terminal(output_cell_id).is_none() =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} attempt {} completed without terminal cell projection",
                        node.node_id, attempt_id
                    )));
                }
                store::AttemptStatus::Completed { .. } | store::AttemptStatus::Failed { .. } => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_terminal_cell_has_completed_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    terminal: &store::CellTerminalProjection,
) -> Result<AttemptId> {
    let certified = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node_id, terminal_attempt_id, schema_id, semantic_type_id) = match terminal {
        store::CellTerminalProjection::Produced {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        }
        | store::CellTerminalProjection::Skipped {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        } => (node_id, attempt_id, schema_id, semantic_type_id),
    };
    if terminal_node_id != &node.node_id
        || certified.producer != spec::CellProducer::Node(node.node_id.clone())
        || schema_id != &certified.schema_id
        || semantic_type_id != &certified.semantic_type_id
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} is not certified terminal evidence for node {}",
            node.output_cell, node.node_id
        )));
    }
    match projections.attempt(&node.node_id, terminal_attempt_id) {
        Some(store::AttemptProjection {
            status: store::AttemptStatus::Completed { output_cell_id },
            ..
        }) if output_cell_id == &node.output_cell => Ok(terminal_attempt_id.clone()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} for node {} lacks matching completed attempt {}",
            node.output_cell, node.node_id, terminal_attempt_id
        ))),
    }
}

pub(crate) fn side_effect_projection_for_attempt<'a>(
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == node.node_id
            && projection.intent.attempt_id == *attempt_id
            && found.replace(projection).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledger projections",
                node.node_id, attempt_id
            )));
        }
    }
    Ok(found)
}

pub(crate) fn validate_side_effect_terminal_evidence(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    if matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )))
    }
}

pub(crate) fn side_effect_artifact_binding(
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    phase: StagedSideEffectArtifactPhase,
) -> StagedArtifactBindingKind {
    StagedArtifactBindingKind::SideEffectEvidence {
        ledger_key,
        invocation_epoch,
        phase,
    }
}

pub(crate) fn validate_runner_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    }
    let (payload_node_id, payload_attempt_id, _, _) =
        side_effect_payload_ref(payload).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    validate_side_effect_ledger_purpose(runtime_spec, projections, node, payload)?;
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} persisted intent for uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            require_capability(
                caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
        }
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            if payload.new_claim_owner == payload.previous_claim_owner {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover reused the previous claim owner",
                    node.node_id
                )));
            }
            if payload.claim_generation <= payload.previous_claim_generation {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover did not increase claim generation",
                    node.node_id
                )));
            }
            if let Some(projection) =
                side_effect_projection_for_attempt(projections, node, attempt_id)?
            {
                let claim = projection.claim.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover requires an active claim",
                        node.node_id
                    ))
                })?;
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover reused the previous fencing token",
                        node.node_id
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_side_effect_resume_output(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Ok(());
    };
    let has_takeover = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
        )
    });
    let has_claim = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::SideEffectClaimed(_)));
    let has_invocation_prepared = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
    });
    let has_invocation_started = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )
    });
    let has_submission_recovery = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectNotSubmittedProven(_)
                | events::KernelEventPayload::SideEffectSubmissionObserved(_)
                | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                | events::KernelEventPayload::SideEffectAmbiguous(_)
        )
    });

    match projection.phase {
        store::SideEffectPhase::Claimed { .. } => {
            if !has_takeover && !has_invocation_prepared {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed claimed ledger {} without takeover or prepared invocation",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationPrepared { .. } => {
            if !has_takeover && !has_invocation_started {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed prepared ledger {} without takeover or invocation start",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::NotSubmittedProven { .. } => {
            if !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationStarted { .. }
        | store::SideEffectPhase::SubmissionUnknown { .. } => {
            if !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::Ambiguous { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run ambiguous ledger {}",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::Failed { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run failed ledger {} on the same attempt",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::IntentPersisted { .. }
        | store::SideEffectPhase::SubmissionObserved { .. }
        | store::SideEffectPhase::ReceiptObserved { .. }
        | store::SideEffectPhase::ConfirmationObserved { .. } => {}
    }

    if has_invocation_started
        && matches!(projection.phase, store::SideEffectPhase::Claimed { .. })
        && !has_takeover
        && !has_invocation_prepared
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} started invocation from stale claim without takeover",
            node.node_id
        )));
    }

    Ok(())
}
