use super::*;
use mfm_certify::{CertifiedRemediationLink, CertifiedSideEffectContract};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoricalSideEffectPhase {
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
    Failed {
        after_not_submitted: bool,
        retryable: bool,
    },
}

impl HistoricalSideEffectPhase {
    fn from_event_kind(kind: events::SideEffectEventKind) -> Option<Self> {
        match kind {
            events::SideEffectEventKind::IntentPersisted => Some(Self::IntentPersisted),
            events::SideEffectEventKind::Claimed | events::SideEffectEventKind::ClaimTakenOver => {
                Some(Self::Claimed)
            }
            events::SideEffectEventKind::ResourceLaneClaimed => Some(Self::ResourceLaneClaimed),
            events::SideEffectEventKind::InvocationPrepared => Some(Self::InvocationPrepared),
            events::SideEffectEventKind::InvocationStarted => Some(Self::InvocationStarted),
            events::SideEffectEventKind::NotSubmittedProven => Some(Self::NotSubmittedProven),
            events::SideEffectEventKind::SubmissionObserved => Some(Self::SubmissionObserved),
            events::SideEffectEventKind::SubmissionUnknown => Some(Self::SubmissionUnknown),
            events::SideEffectEventKind::ReceiptObserved => Some(Self::ReceiptObserved),
            events::SideEffectEventKind::ConfirmationObserved => Some(Self::ConfirmationObserved),
            events::SideEffectEventKind::Ambiguous => Some(Self::Ambiguous),
            events::SideEffectEventKind::Failed => None,
            events::SideEffectEventKind::ResourceLaneReleased => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoricalSideEffectLedger {
    phase: HistoricalSideEffectPhase,
    resource_key: Option<events::ResourceKeyEvidence>,
    purpose: events::SideEffectLedgerPurpose,
    node_id: NodeId,
}

pub(super) fn validate_historical_side_effect_payload(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    if validate_resource_lane_release_payload(spec, run_id, ledgers, payload)? {
        return Ok(());
    }
    let (node_id, attempt_id, ledger_key, pair_id, event_kind) =
        side_effect_payload_ref(payload)
            .ok_or_else(|| invalid_history("expected side-effect payload"))?;
    let phase = match payload {
        events::KernelEventPayload::SideEffectFailed(failed) => HistoricalSideEffectPhase::Failed {
            after_not_submitted: failed.failure_phase
                == events::side_effect::FailurePhase::AfterNotSubmittedProven,
            retryable: failed.retryable,
        },
        _ => {
            let Some(phase) = HistoricalSideEffectPhase::from_event_kind(event_kind) else {
                return Ok(());
            };
            phase
        }
    };
    let node = spec.node(node_id).ok_or_else(|| {
        invalid_history(format!("side-effect record for uncertified node {node_id}"))
    })?;
    let contract_node = side_effect_contract_node_for_payload(spec, node, payload)?;
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(invalid_history(format!(
            "side-effect ledger {} for node {} attempt {} advanced outside an active attempt",
            ledger_key, node_id, attempt_id
        )));
    }
    let contract = CertifiedSideEffectContract::for_node(spec.spec(), &contract_node.node_id)
        .map_err(|error| invalid_history(error.to_string()))?;
    let terminal_policies = SideEffectTerminalPolicies::from_spec(spec.spec())?;
    validate_side_effect_ledger_purpose(&contract, run_id, ledgers, payload, &terminal_policies)?;
    validate_side_effect_resource_claim(&contract, payload)?;

    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(intent) => {
            if intent.scope_id != node.scope_id {
                return Err(invalid_history(format!(
                    "side-effect intent for node {} carries uncertified scope {}",
                    node.node_id, intent.scope_id
                )));
            }
            if !node
                .capability_bindings
                .capabilities
                .iter()
                .any(|capability| {
                    capability.kind == intent.capability_kind
                        && capability.version == intent.capability_version
                })
            {
                return Err(invalid_history(format!(
                    "node {} used uncertified capability {}:{}",
                    node.node_id, intent.capability_kind, intent.capability_version
                )));
            }
            if !node.adapter_bindings.iter().any(|adapter| {
                adapter.adapter_kind == intent.adapter_kind
                    && adapter.adapter_version == intent.adapter_version
            }) {
                return Err(invalid_history(format!(
                    "node {} used uncertified adapter {}:{}",
                    node.node_id, intent.adapter_kind, intent.adapter_version
                )));
            }
            if ledgers
                .insert(
                    pair_id.clone(),
                    HistoricalSideEffectLedger {
                        phase,
                        resource_key: None,
                        purpose: intent.ledger_purpose.clone(),
                        node_id: node.node_id.clone(),
                    },
                )
                .is_some()
            {
                return Err(invalid_history(format!(
                    "side-effect ledger {} persisted intent more than once",
                    ledger_key
                )));
            }
        }
        _ => {
            let ledger = ledgers.get_mut(pair_id).ok_or_else(|| {
                invalid_history(format!(
                    "side-effect ledger {} advanced before intent was persisted",
                    ledger_key
                ))
            })?;
            if let events::KernelEventPayload::ResourceLaneClaimed(claimed) = payload {
                contract
                    .validate_epoch_resource_consistency(
                        ledger.resource_key.as_ref(),
                        Some(&claimed.resource_key),
                    )
                    .map_err(|error| invalid_history(error.to_string()))?;
                ledger.resource_key = Some(claimed.resource_key.clone());
            }
            ledger.phase = phase;
        }
    }
    Ok(())
}

pub(super) fn side_effect_payload_ref(
    payload: &events::KernelEventPayload,
) -> Option<(
    &NodeId,
    &AttemptId,
    &events::SideEffectLedgerKey,
    &SideEffectPairId,
    events::SideEffectEventKind,
)> {
    let emitter = payload.side_effect_emitter_ref()?;
    let ledger = payload.side_effect_ledger_ref()?;
    Some((
        emitter.node_id,
        emitter.attempt_id,
        ledger.ledger_key,
        ledger.pair_id,
        ledger.kind,
    ))
}

fn side_effect_contract_node_for_payload<'a>(
    spec: &HistorySpec<'a>,
    node: &'a spec::NodeSpec,
    payload: &events::KernelEventPayload,
) -> Result<&'a spec::NodeSpec> {
    if node.side_effect.is_some() {
        return Ok(node);
    }
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(invalid_history(format!(
            "non-side-effect node {} emitted side-effect payload",
            node.node_id
        )));
    };
    let ledger = payload
        .side_effect_ledger_ref()
        .ok_or_else(|| invalid_history("expected side-effect payload"))?;
    if ledger.pair_id != &verify.pair_id || ledger.pair_role != events::SideEffectPairRole::Verify {
        return Err(invalid_history(format!(
            "side-effect verify node {} emitted outside certified pair {}",
            node.node_id, verify.pair_id
        )));
    }
    spec.node(&verify.submit_node_id).ok_or_else(|| {
        invalid_history(format!(
            "side-effect verify node {} references missing submit node {}",
            node.node_id, verify.submit_node_id
        ))
    })
}

fn validate_side_effect_ledger_purpose(
    contract: &CertifiedSideEffectContract,
    run_id: &RunId,
    ledgers: &BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<()> {
    let purpose = payload
        .side_effect_ledger_ref()
        .map(|side_effect| side_effect.ledger_purpose)
        .ok_or_else(|| invalid_history("expected side-effect payload"))?;
    contract
        .validate_ledger_purpose(purpose)
        .map_err(|error| invalid_history(error.to_string()))?;
    let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } = purpose else {
        return Ok(());
    };
    let forward = ledgers.get(forward_pair_id);
    contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: run_id,
            ledger_purpose: purpose,
            forward_run_id: forward.map(|_| run_id),
            forward_node_id: forward.map(|item| &item.node_id),
            forward_ledger_purpose: forward.map(|item| &item.purpose),
            forward_terminal: forward
                .map(|item| {
                    terminal_policies
                        .require(forward_pair_id)
                        .map(|policy| phase_satisfies_policy(policy, item.phase))
                })
                .transpose()?
                .unwrap_or(false),
        })
        .map_err(|error| invalid_history(error.to_string()))
}

fn validate_resource_lane_release_payload(
    spec: &HistorySpec<'_>,
    run_id: &RunId,
    ledgers: &BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
) -> Result<bool> {
    let Some(resource) = payload.resource_lane_authority_ref() else {
        return Ok(false);
    };
    if resource.release_authority.is_none() {
        return Ok(false);
    }
    if resource.ledger.pair_role != events::SideEffectPairRole::Verify {
        return Err(invalid_history(format!(
            "resource-lane release for pair {} must use verify role",
            resource.ledger.pair_id
        )));
    }
    let pair = spec
        .spec()
        .side_effect_verify_pair_for_pair_id(resource.ledger.pair_id)
        .map_err(|error| invalid_history(error.to_string()))?;
    let contract = CertifiedSideEffectContract::for_node(spec.spec(), &pair.submit_node.node_id)
        .map_err(|error| invalid_history(error.to_string()))?;
    validate_side_effect_ledger_purpose(
        &contract,
        run_id,
        ledgers,
        payload,
        &SideEffectTerminalPolicies::from_spec(spec.spec())?,
    )?;
    Ok(true)
}

fn validate_side_effect_resource_claim(
    contract: &CertifiedSideEffectContract,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    match payload {
        events::KernelEventPayload::ResourceLaneClaimed(payload) => contract
            .validate_resource_key(Some(&payload.resource_key))
            .map_err(|error| invalid_history(error.to_string()))?,
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => contract
            .validate_resource_key(payload.resource_key.as_ref())
            .map_err(|error| invalid_history(error.to_string()))?,
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => contract
            .validate_touched_set(payload.resource_touched_set.as_ref())
            .map_err(|error| invalid_history(error.to_string()))?,
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => contract
            .validate_touched_set(payload.resource_touched_set.as_ref())
            .map_err(|error| invalid_history(error.to_string()))?,
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_historical_side_effect_terminal(
    spec: &HistorySpec<'_>,
    ledgers: &BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    skipped: bool,
) -> Result<()> {
    if node.side_effect.is_some() {
        let pair_id = spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .ok_or_else(|| {
                invalid_history(format!(
                    "side-effect node {} is missing certified verify pair",
                    node.node_id
                ))
            })?;
        let ledger = historical_ledger(ledgers, pair_id)?;
        if skipped {
            if phase_has_submission_result(ledger.phase) {
                return Ok(());
            }
            return Err(invalid_history(format!(
                "side-effect node {} attempt {} skipped before a submission result",
                node.node_id, attempt_id
            )));
        }
        let policies = SideEffectTerminalPolicies::from_spec(spec.spec())?;
        if phase_satisfies_policy(policies.require(pair_id)?, ledger.phase) {
            return Ok(());
        }
        return Err(invalid_history(format!(
            "side-effect node {} attempt {} produced before terminal evidence",
            node.node_id, attempt_id
        )));
    }
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Ok(());
    };
    let ledger = historical_ledger(ledgers, &verify.pair_id)?;
    let submit = spec.node(&verify.submit_node_id).ok_or_else(|| {
        invalid_history(format!(
            "verify node {} references missing submit node {}",
            node.node_id, verify.submit_node_id
        ))
    })?;
    let contract = submit.side_effect.as_ref().ok_or_else(|| {
        invalid_history(format!(
            "verify node {} references non-side-effect submit node {}",
            node.node_id, submit.node_id
        ))
    })?;
    if phase_satisfies_policy(
        SideEffectTerminalPolicy::from_verification(&contract.verification),
        ledger.phase,
    ) {
        Ok(())
    } else {
        Err(invalid_history(format!(
            "verify node {} attempt {} produced before terminal evidence",
            node.node_id, attempt_id
        )))
    }
}

pub(super) fn validate_historical_side_effect_failure(
    spec: &HistorySpec<'_>,
    ledgers: &BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let pair_id = if node.side_effect.is_some() {
        let pair_id = spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .ok_or_else(|| invalid_history("side-effect node is missing certified verify pair"))?;
        Some(pair_id)
    } else if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
        Some(&verify.pair_id)
    } else {
        None
    };
    let Some(pair_id) = pair_id else {
        return Ok(());
    };
    let Some(ledger) = ledgers.get(pair_id) else {
        return Ok(());
    };
    if matches!(
        ledger.phase,
        HistoricalSideEffectPhase::Failed { .. } | HistoricalSideEffectPhase::Ambiguous
    ) {
        Ok(())
    } else {
        Err(invalid_history(format!(
            "side-effect node {} attempt {} failed without terminal side-effect evidence",
            node.node_id, attempt_id
        )))
    }
}

fn historical_ledger<'a>(
    ledgers: &'a BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    pair_id: &SideEffectPairId,
) -> Result<&'a HistoricalSideEffectLedger> {
    ledgers.get(pair_id).ok_or_else(|| {
        invalid_history(format!(
            "side-effect pair {} lacks ledger evidence",
            pair_id
        ))
    })
}

fn phase_has_submission_result(phase: HistoricalSideEffectPhase) -> bool {
    matches!(
        phase,
        HistoricalSideEffectPhase::NotSubmittedProven
            | HistoricalSideEffectPhase::SubmissionObserved
            | HistoricalSideEffectPhase::SubmissionUnknown
            | HistoricalSideEffectPhase::ReceiptObserved
            | HistoricalSideEffectPhase::ConfirmationObserved
            | HistoricalSideEffectPhase::Ambiguous
            | HistoricalSideEffectPhase::Failed { .. }
    )
}

fn phase_satisfies_policy(
    policy: SideEffectTerminalPolicy,
    phase: HistoricalSideEffectPhase,
) -> bool {
    match policy {
        SideEffectTerminalPolicy::Receipt => matches!(
            phase,
            HistoricalSideEffectPhase::ReceiptObserved
                | HistoricalSideEffectPhase::ConfirmationObserved
        ),
        SideEffectTerminalPolicy::Confirmation => {
            phase == HistoricalSideEffectPhase::ConfirmationObserved
        }
    }
}

pub(super) fn derive_uncompleted_saga_mode(
    spec: &HistorySpec<'_>,
    ledgers: &BTreeMap<SideEffectPairId, HistoricalSideEffectLedger>,
    saga_engaged: bool,
    manual: Option<events::ManualResolutionOutcome>,
) -> Result<(RunMode, Option<ManualBlockReason>)> {
    if let Some(outcome) = manual {
        return Ok((
            match outcome {
                events::ManualResolutionOutcome::ConfirmRemediated => RunMode::ManuallyResolved,
                events::ManualResolutionOutcome::FailWithoutAcdcClaim => {
                    RunMode::FailedWithoutAcdcClaim
                }
            },
            None,
        ));
    }
    let policies = SideEffectTerminalPolicies::from_spec(spec.spec())?;
    let forwards = ledgers
        .iter()
        .filter(|(_, ledger)| matches!(ledger.purpose, events::SideEffectLedgerPurpose::Forward))
        .collect::<Vec<_>>();
    let mut forward_quiescent = true;
    for (pair_id, ledger) in &forwards {
        if phase_crossed_boundary(ledger.phase)
            && !phase_is_quiescent(ledger.phase, policies.require(pair_id)?)
        {
            forward_quiescent = false;
            break;
        }
    }
    if !saga_engaged || !forward_quiescent {
        return Ok((RunMode::Forward, None));
    }
    let has_forward_boundary = forwards
        .iter()
        .any(|(_, ledger)| phase_crossed_boundary(ledger.phase));
    let mut owed = Vec::new();
    let mut unresolved = None;
    for (pair_id, ledger) in &forwards {
        match classify_forward(ledger.phase, policies.require(pair_id)?) {
            ForwardLedgerClassification::Owed => owed.push((*pair_id).clone()),
            ForwardLedgerClassification::Unresolvable => {
                unresolved.get_or_insert(ManualBlockReason::ForwardAmbiguous);
            }
            ForwardLedgerClassification::Pending | ForwardLedgerClassification::NothingOwed => {}
        }
    }
    let mut all_owed_closed = !owed.is_empty();
    for forward_pair in &owed {
        let remediation = ledgers.iter().find(|(_, ledger)| {
            matches!(
                &ledger.purpose,
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id }
                    if forward_pair_id == forward_pair
            )
        });
        let Some((remediation_pair, remediation)) = remediation else {
            all_owed_closed = false;
            continue;
        };
        if !phase_satisfies_policy(policies.require(remediation_pair)?, remediation.phase) {
            all_owed_closed = false;
        }
        match remediation.phase {
            HistoricalSideEffectPhase::Ambiguous => {
                unresolved.get_or_insert(ManualBlockReason::RemediationAmbiguous);
            }
            HistoricalSideEffectPhase::Failed {
                retryable: false, ..
            } => {
                unresolved.get_or_insert(ManualBlockReason::RemediationFailed);
            }
            _ => {}
        }
    }
    if !has_forward_boundary || (owed.is_empty() && unresolved.is_none()) {
        return Ok((RunMode::FailedWithoutAcdcClaim, None));
    }
    match &spec.spec().saga {
        spec::SagaPolicySpec::NoSideEffects | spec::SagaPolicySpec::FailWithoutAcdcClaim => {
            Ok((RunMode::FailedWithoutAcdcClaim, None))
        }
        spec::SagaPolicySpec::ManualResolution { .. } => Ok((
            RunMode::ManualBlocked,
            Some(ManualBlockReason::PolicyManualResolution),
        )),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved,
        } => {
            if let Some(reason) = unresolved {
                match on_remediation_unresolved {
                    spec::RemediationUnresolvedSpec::ManualResolution { .. } => {
                        Ok((RunMode::ManualBlocked, Some(reason)))
                    }
                    spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                        Ok((RunMode::FailedWithoutAcdcClaim, None))
                    }
                }
            } else if all_owed_closed {
                Ok((RunMode::Compensated, None))
            } else {
                Ok((RunMode::Remediating, None))
            }
        }
    }
}

fn classify_forward(
    phase: HistoricalSideEffectPhase,
    policy: SideEffectTerminalPolicy,
) -> ForwardLedgerClassification {
    match phase {
        HistoricalSideEffectPhase::IntentPersisted
        | HistoricalSideEffectPhase::Claimed
        | HistoricalSideEffectPhase::ResourceLaneClaimed
        | HistoricalSideEffectPhase::InvocationPrepared
        | HistoricalSideEffectPhase::NotSubmittedProven
        | HistoricalSideEffectPhase::Failed { .. } => ForwardLedgerClassification::NothingOwed,
        HistoricalSideEffectPhase::InvocationStarted
        | HistoricalSideEffectPhase::SubmissionObserved
        | HistoricalSideEffectPhase::SubmissionUnknown => ForwardLedgerClassification::Pending,
        HistoricalSideEffectPhase::ReceiptObserved
        | HistoricalSideEffectPhase::ConfirmationObserved => {
            if phase_satisfies_policy(policy, phase) {
                ForwardLedgerClassification::Owed
            } else {
                ForwardLedgerClassification::Pending
            }
        }
        HistoricalSideEffectPhase::Ambiguous => ForwardLedgerClassification::Unresolvable,
    }
}

fn phase_crossed_boundary(phase: HistoricalSideEffectPhase) -> bool {
    matches!(
        phase,
        HistoricalSideEffectPhase::InvocationStarted
            | HistoricalSideEffectPhase::SubmissionObserved
            | HistoricalSideEffectPhase::SubmissionUnknown
            | HistoricalSideEffectPhase::ReceiptObserved
            | HistoricalSideEffectPhase::ConfirmationObserved
            | HistoricalSideEffectPhase::Ambiguous
            | HistoricalSideEffectPhase::NotSubmittedProven
            | HistoricalSideEffectPhase::Failed {
                after_not_submitted: true,
                ..
            }
    )
}

fn phase_is_quiescent(phase: HistoricalSideEffectPhase, policy: SideEffectTerminalPolicy) -> bool {
    match phase {
        HistoricalSideEffectPhase::NotSubmittedProven
        | HistoricalSideEffectPhase::ConfirmationObserved
        | HistoricalSideEffectPhase::Ambiguous
        | HistoricalSideEffectPhase::Failed { .. } => true,
        HistoricalSideEffectPhase::ReceiptObserved => phase_satisfies_policy(policy, phase),
        HistoricalSideEffectPhase::IntentPersisted
        | HistoricalSideEffectPhase::Claimed
        | HistoricalSideEffectPhase::ResourceLaneClaimed
        | HistoricalSideEffectPhase::InvocationPrepared
        | HistoricalSideEffectPhase::InvocationStarted
        | HistoricalSideEffectPhase::SubmissionObserved
        | HistoricalSideEffectPhase::SubmissionUnknown => false,
    }
}

pub(super) fn node_uses_side_effect_terminal_validation(node: &spec::NodeSpec) -> bool {
    node.side_effect.is_some()
        || matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
        )
}

pub(super) fn validate_atomic_resource_lane_release_pairs(
    spec: &HistorySpec<'_>,
    records: &[KernelEventEnvelope],
) -> Result<()> {
    let terminal_policies = SideEffectTerminalPolicies::from_spec(spec.spec())?;
    for commit in committed_journal_commits(records) {
        validate_manual_resolution_release_batch(commit.events)?;
        for (release_index, event) in commit.events.iter().enumerate() {
            let events::KernelEventPayload::ResourceLaneReleased(release) = event.payload() else {
                continue;
            };
            match release.release_authority {
                events::ResourceLaneReleaseAuthority::ManualResolution => {
                    let followed_by_manual_resolution = commit.events
                        [release_index.saturating_add(1)..]
                        .iter()
                        .any(|candidate| {
                            matches!(
                                candidate.payload(),
                                events::KernelEventPayload::ManualResolutionRecorded(_)
                            )
                        });
                    if !followed_by_manual_resolution {
                        return Err(invalid_history(
                            "manual resource-lane release requires adjacent ManualResolutionRecorded",
                        ));
                    }
                }
                events::ResourceLaneReleaseAuthority::VerifyTerminal => {
                    let terminal_policy = terminal_policies.require(&release.pair_id)?;
                    let followed_by_terminal =
                        commit
                            .events
                            .get(release_index + 1)
                            .is_some_and(|candidate| {
                                side_effect_terminal_matches_release(
                                    candidate.payload(),
                                    release,
                                    terminal_policy,
                                )
                            });
                    if !followed_by_terminal {
                        return Err(invalid_history(
                            "verify-terminal resource-lane release must immediately precede terminal side-effect evidence",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_manual_resolution_release_batch(records: &[KernelEventEnvelope]) -> Result<()> {
    let manual_resolution_index = records.iter().position(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ManualResolutionRecorded(_)
        )
    });
    let contains_manual_release = records.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
                release_authority: events::ResourceLaneReleaseAuthority::ManualResolution,
                ..
            })
        )
    });
    if manual_resolution_index.is_none() && !contains_manual_release {
        return Ok(());
    }
    let Some(manual_resolution_index) = manual_resolution_index else {
        return Err(invalid_history(
            "manual resource-lane release requires adjacent ManualResolutionRecorded",
        ));
    };
    if manual_resolution_index + 1 != records.len()
        || records[..manual_resolution_index].iter().any(|event| {
            !matches!(
                event.payload(),
                events::KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
                    release_authority: events::ResourceLaneReleaseAuthority::ManualResolution,
                    ..
                })
            )
        })
    {
        return Err(invalid_history(
            "manual resource-lane release requires adjacent ManualResolutionRecorded",
        ));
    }
    Ok(())
}

fn side_effect_terminal_matches_release(
    payload: &events::KernelEventPayload,
    release: &events::ResourceLaneReleased,
    terminal_policy: SideEffectTerminalPolicy,
) -> bool {
    if !payload.is_side_effect_terminal_disposition() {
        return false;
    }
    let Some(terminal) = payload.side_effect_ledger_ref() else {
        return false;
    };
    terminal.ledger_key == &release.ledger_key
        && terminal.ledger_purpose == &release.ledger_purpose
        && terminal.pair_id == &release.pair_id
        && terminal.invocation_epoch == Some(release.invocation_epoch)
        && side_effect_terminal_release_role_allowed(
            terminal.kind,
            terminal.pair_role,
            release.pair_role,
        )
        && side_effect_terminal_release_policy_allowed(terminal.kind, terminal_policy)
}

fn side_effect_terminal_release_role_allowed(
    terminal_kind: events::SideEffectEventKind,
    terminal_role: events::SideEffectPairRole,
    release_role: events::SideEffectPairRole,
) -> bool {
    if release_role != events::SideEffectPairRole::Verify {
        return false;
    }
    match terminal_kind {
        events::SideEffectEventKind::NotSubmittedProven => matches!(
            terminal_role,
            events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
        ),
        events::SideEffectEventKind::ReceiptObserved
        | events::SideEffectEventKind::ConfirmationObserved => {
            terminal_role == events::SideEffectPairRole::Verify
        }
        events::SideEffectEventKind::Failed => matches!(
            terminal_role,
            events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
        ),
        _ => false,
    }
}

fn side_effect_terminal_release_policy_allowed(
    terminal_kind: events::SideEffectEventKind,
    terminal_policy: SideEffectTerminalPolicy,
) -> bool {
    match terminal_kind {
        events::SideEffectEventKind::ReceiptObserved => {
            terminal_policy == SideEffectTerminalPolicy::Receipt
        }
        events::SideEffectEventKind::ConfirmationObserved
        | events::SideEffectEventKind::NotSubmittedProven
        | events::SideEffectEventKind::Failed => true,
        _ => false,
    }
}

pub(super) fn validate_atomic_side_effect_failure_pairs(
    spec: &HistorySpec<'_>,
    known_side_effect_pairs: &BTreeSet<SideEffectPairId>,
    records: &[KernelEventEnvelope],
) -> Result<()> {
    let mut known_side_effect_pairs = known_side_effect_pairs.clone();
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in records {
        if let events::KernelEventPayload::SideEffectIntentPersisted(payload) = event.payload() {
            known_side_effect_pairs.insert(payload.pair_id.clone());
        }
        if let Some(side_effect) = event.payload().side_effect_ref() {
            let retryable = match side_effect.kind {
                events::SideEffectEventKind::Failed => match event.payload() {
                    events::KernelEventPayload::SideEffectFailed(payload) => {
                        Some(payload.retryable)
                    }
                    _ => {
                        return Err(invalid_history(
                            "side-effect failure kind does not match payload variant",
                        ));
                    }
                },
                events::SideEffectEventKind::Ambiguous => Some(false),
                _ => None,
            };
            if let Some(retryable) = retryable {
                side_effect_failures.insert(
                    (
                        event.seq(),
                        side_effect.node_id.clone(),
                        side_effect.attempt_id.clone(),
                    ),
                    retryable,
                );
            }
        }
        if let events::KernelEventPayload::StateAttemptFailed(payload) = event.payload() {
            let node = spec.node(&payload.node_id).ok_or_else(|| {
                invalid_history(format!(
                    "attempt failed for uncertified node {}",
                    payload.node_id
                ))
            })?;
            if node_uses_side_effect_terminal_validation(node) {
                let pair_id = side_effect_pair_id_for_node(spec, node)?.ok_or_else(|| {
                    invalid_history(format!(
                        "side-effect node {} is missing certified pair authority",
                        node.node_id
                    ))
                })?;
                if known_side_effect_pairs.contains(&pair_id) {
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
    }
    for (failure, retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(actual) if actual == retryable => {}
            Some(_) => {
                return Err(invalid_history(format!(
                    "terminal side-effect evidence for node {} attempt {} disagrees with attempt retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(invalid_history(format!(
                    "terminal side-effect evidence for node {} attempt {} lacks StateAttemptFailed in the same batch",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(invalid_history(format!(
                "side-effect attempt failure for node {} attempt {} lacks same-batch terminal evidence",
                failure.1, failure.2
            )));
        }
    }
    Ok(())
}

fn side_effect_pair_id_for_node(
    spec: &HistorySpec<'_>,
    node: &spec::NodeSpec,
) -> Result<Option<SideEffectPairId>> {
    if node.side_effect.is_some() {
        return spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| invalid_history("side-effect node is missing certified verify pair"));
    }
    Ok(
        if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
            Some(verify.pair_id.clone())
        } else {
            None
        },
    )
}
