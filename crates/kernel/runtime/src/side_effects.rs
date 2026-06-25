use std::collections::{BTreeMap, BTreeSet};

use mfm_certify::{CertifiedRemediationLink, CertifiedSideEffectContract};
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, RunId, SideEffectPairId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{StagedArtifactBindingKind, StagedSideEffectArtifactPhase};
use crate::side_effect_lifecycle::SideEffectLifecycle;
use crate::{
    require_adapter, require_attempt, require_capability, CertifiedRuntimeCapabilities,
    CertifiedRuntimeSpec, Result, RuntimeError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoricalSideEffectPhase {
    IntentPersisted,
    Claimed,
    ResourceLaneClaimed,
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

impl From<events::SideEffectEventKind> for HistoricalSideEffectPhase {
    fn from(kind: events::SideEffectEventKind) -> Self {
        match kind {
            events::SideEffectEventKind::IntentPersisted => Self::IntentPersisted,
            events::SideEffectEventKind::Claimed | events::SideEffectEventKind::ClaimTakenOver => {
                Self::Claimed
            }
            events::SideEffectEventKind::ResourceLaneClaimed => Self::ResourceLaneClaimed,
            events::SideEffectEventKind::InvocationPrepared => Self::InvocationPrepared,
            events::SideEffectEventKind::InvocationStarted => Self::InvocationStarted,
            events::SideEffectEventKind::NotSubmittedProven => Self::NotSubmittedProven,
            events::SideEffectEventKind::SubmissionObserved => Self::SubmissionObserved,
            events::SideEffectEventKind::SubmissionUnknown => Self::SubmissionUnknown,
            events::SideEffectEventKind::ReceiptObserved => Self::ReceiptObserved,
            events::SideEffectEventKind::ConfirmationObserved => Self::ConfirmationObserved,
            events::SideEffectEventKind::Ambiguous => Self::Ambiguous,
            events::SideEffectEventKind::Failed => Self::Failed,
            events::SideEffectEventKind::ResourceLaneReleased => Self::ResourceLaneClaimed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoricalSideEffectLedger {
    node_id: NodeId,
    attempt_id: AttemptId,
    pair_id: Option<SideEffectPairId>,
    phase: HistoricalSideEffectPhase,
    resource_key: Option<events::ResourceKeyEvidence>,
}

pub(crate) fn validate_historical_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let (node_id, attempt_id, ledger_key, pair_id, phase) = side_effect_payload_ref(payload)
        .ok_or_else(|| RuntimeError::InvalidRunStream("expected side-effect payload".to_owned()))?;
    let node = runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("side-effect event for uncertified node {node_id}"))
    })?;
    let contract_node = side_effect_contract_node_for_payload(runtime_spec, node, payload)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect ledger {} for node {} attempt {} was recorded outside an active started attempt",
            ledger_key, node_id, attempt_id
        )));
    }
    let contract = certified_side_effect_contract(runtime_spec, contract_node)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    validate_side_effect_ledger_purpose(&contract, projections, run_id, payload)
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    validate_side_effect_resource_claim(&contract, payload)
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
                        pair_id: pair_id.cloned(),
                        phase,
                        resource_key: None,
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
            let payload_pair_role = side_effect_payload_pair_role(payload);
            let verify_pair_event = payload_pair_role == Some(events::SideEffectPairRole::Verify)
                && pair_id.is_some()
                && ledger.pair_id.as_ref() == pair_id;
            if !verify_pair_event
                && (ledger.node_id != *node_id || ledger.attempt_id != *attempt_id)
            {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed node or attempt authority",
                    ledger_key
                )));
            }
            if ledger.pair_id.as_ref() != pair_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed pair authority",
                    ledger_key
                )));
            }
            if let events::KernelEventPayload::ResourceLaneClaimed(payload) = payload {
                contract
                    .validate_epoch_resource_consistency(
                        ledger.resource_key.as_ref(),
                        Some(&payload.resource_key),
                    )
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                ledger.resource_key = Some(payload.resource_key.clone());
            }
            if matches!(payload, events::KernelEventPayload::ResourceLaneReleased(_)) {
                return Ok(());
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
    Option<&SideEffectPairId>,
    HistoricalSideEffectPhase,
)> {
    payload.side_effect_ref().map(|side_effect| {
        (
            side_effect.node_id,
            side_effect.attempt_id,
            side_effect.ledger_key,
            side_effect.pair_id,
            HistoricalSideEffectPhase::from(side_effect.kind),
        )
    })
}

fn side_effect_payload_ledger_purpose(
    payload: &events::KernelEventPayload,
) -> Option<&events::SideEffectLedgerPurpose> {
    payload
        .side_effect_ref()
        .map(|side_effect| side_effect.ledger_purpose)
}

fn side_effect_payload_pair_role(
    payload: &events::KernelEventPayload,
) -> Option<events::SideEffectPairRole> {
    payload
        .side_effect_ref()
        .and_then(|side_effect| side_effect.pair_role)
}

fn certified_side_effect_contract(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> mfm_certify::Result<CertifiedSideEffectContract> {
    CertifiedSideEffectContract::for_node(runtime_spec.spec(), &node.node_id)
}

fn side_effect_contract_node_for_payload<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &'a spec::NodeSpec,
    payload: &events::KernelEventPayload,
) -> Result<&'a spec::NodeSpec> {
    if node.side_effect.is_some() {
        return Ok(node);
    }
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    };
    let Some(side_effect) = payload.side_effect_ref() else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "expected side-effect payload".to_owned(),
        ));
    };
    if side_effect.pair_id != Some(&verify.pair_id)
        || side_effect.pair_role != Some(events::SideEffectPairRole::Verify)
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} emitted payload outside certified pair {}",
            node.node_id, verify.pair_id
        )));
    }
    runtime_spec.node(&verify.submit_node_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "side-effect verify node {} references missing submit node {}",
            node.node_id, verify.submit_node_id
        ))
    })
}

fn validate_side_effect_ledger_purpose(
    contract: &CertifiedSideEffectContract,
    projections: &store::ProjectionSnapshot,
    run_id: &RunId,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let purpose = side_effect_payload_ledger_purpose(payload).ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
    })?;
    contract
        .validate_ledger_purpose(purpose)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let events::SideEffectLedgerPurpose::Remediation {
        forward_ledger_key,
        forward_pair_id,
    } = purpose
    else {
        return Ok(());
    };
    let forward = projections.side_effect_for_run(run_id, forward_ledger_key);
    if let Some(forward_pair_id) = forward_pair_id {
        let pair_forward = projections
            .side_effect_for_pair(run_id, forward_pair_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if pair_forward.map(|projection| &projection.ledger_key)
            != forward.map(|projection| &projection.ledger_key)
        {
            return Err(RuntimeError::InvalidRunnerOutput(
                "remediation forward pair does not match forward ledger key".to_owned(),
            ));
        }
    }
    contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: run_id,
            ledger_purpose: purpose,
            forward_run_id: forward.map(|projection| &projection.run_id),
            forward_node_id: forward.map(|projection| &projection.intent.node_id),
            forward_ledger_purpose: forward.map(|projection| &projection.ledger_purpose),
            forward_confirmed: forward.is_some_and(|projection| {
                matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                )
            }),
        })
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

pub(crate) fn validate_historical_side_effect_terminal(
    runtime_spec: &CertifiedRuntimeSpec,
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    terminal_skipped: bool,
) -> Result<()> {
    if node.side_effect.is_some() {
        let ledger = historical_side_effect_ledger_for_attempt(ledgers, &node.node_id, attempt_id)?;
        if terminal_skipped {
            if historical_phase_has_submission_result(ledger.phase) {
                return Ok(());
            }
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} skipped output before a submission result",
                node.node_id, attempt_id
            )));
        }
        if ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved {
            return Ok(());
        }
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )));
    }
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Ok(());
    };
    let ledger = historical_side_effect_ledger_for_pair(ledgers, &verify.pair_id)?;
    let submit = runtime_spec.node(&verify.submit_node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect verify node {} references missing submit node {}",
            node.node_id, verify.submit_node_id
        ))
    })?;
    let Some(contract) = &submit.side_effect else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect verify node {} references non-side-effect submit node {}",
            node.node_id, submit.node_id
        )));
    };
    let ok = match &contract.verification {
        spec::SideEffectVerificationSpec::Receipt => matches!(
            ledger.phase,
            HistoricalSideEffectPhase::ReceiptObserved
                | HistoricalSideEffectPhase::ConfirmationObserved
        ),
        spec::SideEffectVerificationSpec::Finalized { .. } => {
            ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved
        }
    };
    if ok {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect verify node {} attempt {} produced output before certified terminal evidence",
            node.node_id, attempt_id
        )))
    }
}

pub(crate) fn validate_historical_side_effect_failure(
    runtime_spec: &CertifiedRuntimeSpec,
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = if node.side_effect.is_some() {
        historical_side_effect_ledger_for_attempt(ledgers, &node.node_id, attempt_id)?
    } else if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
        let _ = runtime_spec;
        historical_side_effect_ledger_for_pair(ledgers, &verify.pair_id)?
    } else {
        return Ok(());
    };
    if matches!(
        ledger.phase,
        HistoricalSideEffectPhase::Failed | HistoricalSideEffectPhase::Ambiguous
    ) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} failed without terminal side-effect evidence",
            node.node_id, attempt_id
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

fn historical_side_effect_ledger_for_pair<'a>(
    ledgers: &'a BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    pair_id: &SideEffectPairId,
) -> Result<&'a HistoricalSideEffectLedger> {
    let mut found = None;
    for ledger in ledgers.values() {
        if ledger.pair_id.as_ref() == Some(pair_id) && found.replace(ledger).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect pair {} has multiple ledgers in history",
                pair_id
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect pair {} lacks ledger evidence",
            pair_id
        ))
    })
}

fn historical_phase_has_submission_result(phase: HistoricalSideEffectPhase) -> bool {
    matches!(
        phase,
        HistoricalSideEffectPhase::NotSubmittedProven
            | HistoricalSideEffectPhase::SubmissionObserved
            | HistoricalSideEffectPhase::SubmissionUnknown
            | HistoricalSideEffectPhase::ReceiptObserved
            | HistoricalSideEffectPhase::ConfirmationObserved
            | HistoricalSideEffectPhase::Ambiguous
            | HistoricalSideEffectPhase::Failed
    )
}

pub(crate) fn node_uses_side_effect_terminal_validation(node: &spec::NodeSpec) -> bool {
    node.side_effect.is_some()
        || matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
        )
}

pub(crate) fn validate_atomic_side_effect_failure_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in stream {
        if let Some(side_effect) = event.payload().side_effect_ref() {
            match side_effect.kind {
                events::SideEffectEventKind::Failed => {
                    let events::KernelEventPayload::SideEffectFailed(payload) = event.payload()
                    else {
                        unreachable!("side-effect kind came from payload variant");
                    };
                    side_effect_failures.insert(
                        (
                            event.seq(),
                            side_effect.node_id.clone(),
                            side_effect.attempt_id.clone(),
                        ),
                        payload.retryable,
                    );
                }
                events::SideEffectEventKind::Ambiguous => {
                    side_effect_failures.insert(
                        (
                            event.seq(),
                            side_effect.node_id.clone(),
                            side_effect.attempt_id.clone(),
                        ),
                        false,
                    );
                }
                _ => {}
            }
        }
        if let events::KernelEventPayload::StateAttemptFailed(payload) = event.payload() {
            let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "attempt failed for uncertified node {}",
                    payload.node_id
                ))
            })?;
            if node_uses_side_effect_terminal_validation(node) {
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
    }
    for (failure, side_effect_retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
            Some(_) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "terminal side-effect evidence for node {} attempt {} disagrees with StateAttemptFailed retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "terminal side-effect evidence for node {} attempt {} lacks StateAttemptFailed in the same commit",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect attempt failure for node {} attempt {} lacks terminal side-effect evidence in the same commit",
                failure.1, failure.2
            )));
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
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let contract_node = side_effect_contract_node_for_payload(runtime_spec, node, payload)?;
    let (payload_node_id, payload_attempt_id, _, _, _) = side_effect_payload_ref(payload)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    let contract = certified_side_effect_contract(runtime_spec, contract_node)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    validate_side_effect_ledger_purpose(&contract, projections, run_id, payload)?;
    validate_side_effect_resource_claim(&contract, payload)?;
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
                SideEffectLifecycle::projection_for_attempt(projections, node, attempt_id)?
            {
                projection
                    .ledger_state()
                    .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
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

fn validate_side_effect_resource_claim(
    contract: &CertifiedSideEffectContract,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    match payload {
        events::KernelEventPayload::ResourceLaneClaimed(payload) => {
            contract
                .validate_resource_key(Some(&payload.resource_key))
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        }
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            contract
                .validate_resource_key(payload.resource_key.as_ref())
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
            contract
                .validate_touched_set(payload.resource_touched_set.as_ref())
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        }
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            contract
                .validate_touched_set(payload.resource_touched_set.as_ref())
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        }
        _ => {}
    }
    Ok(())
}
