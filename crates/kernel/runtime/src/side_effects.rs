use mfm_certify::{CertifiedRemediationLink, CertifiedSideEffectContract};
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, RunId, SideEffectPairId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::side_effect_lifecycle::side_effect_for_attempt;
use crate::spec_authority::CurrentSpecRead;
use crate::{
    require_adapter, require_attempt, require_capability, CertifiedRuntimeCapabilities, Result,
    RuntimeError,
};

fn side_effect_payload_ref(
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

fn side_effect_contract_node_for_payload<'a, S>(
    runtime_spec: &'a S,
    node: &'a spec::NodeSpec,
    payload: &events::KernelEventPayload,
) -> Result<&'a spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    if node.side_effect.is_some() {
        return Ok(node);
    }
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    };
    let Some(side_effect) = payload.side_effect_ledger_ref() else {
        return Err(RuntimeError::InvalidRunnerOutput(
            "expected side-effect payload".to_owned(),
        ));
    };
    if side_effect.pair_id != &verify.pair_id
        || side_effect.pair_role != events::SideEffectPairRole::Verify
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

pub(crate) fn validate_terminal_cell_has_completed_attempt<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    node: &spec::NodeSpec,
    terminal: &store::current_lifecycle::CurrentCellRef<'_>,
) -> Result<AttemptId>
where
    S: CurrentSpecRead + ?Sized,
{
    let certified = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node_id, terminal_attempt_id, schema_id, semantic_type_id) =
        if let Some(produced) = terminal.produced() {
            (
                produced.node_id(),
                produced.attempt_id(),
                produced.schema_id(),
                produced.semantic_type_id(),
            )
        } else if let Some(skipped) = terminal.skipped() {
            (
                skipped.node_id(),
                skipped.attempt_id(),
                skipped.schema_id(),
                skipped.semantic_type_id(),
            )
        } else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "terminal cell {} has no produced or skipped state",
                node.output_cell
            )));
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
    match lifecycle
        .attempt(&node.node_id, terminal_attempt_id)
        .map(|attempt| attempt.status())
    {
        Some(store::current_lifecycle::CurrentAttemptStatusRef::Completed { output_cell_id })
            if output_cell_id == &node.output_cell =>
        {
            Ok(terminal_attempt_id.clone())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} for node {} lacks matching completed attempt {}",
            node.output_cell, node.node_id, terminal_attempt_id
        ))),
    }
}

pub(crate) fn validate_runner_side_effect_payload<S>(
    runtime_spec: &S,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    payload: &events::KernelEventPayload,
) -> Result<()>
where
    S: CurrentSpecRead + ?Sized,
{
    if validate_current_resource_lane_release_payload(
        runtime_spec,
        run_id,
        lifecycle,
        payload,
        Some(events::ResourceLaneReleaseAuthority::VerifyTerminal),
        Some(node),
    )? {
        return Ok(());
    }
    let contract_node = side_effect_contract_node_for_payload(runtime_spec, node, payload)?;
    let (payload_node_id, payload_attempt_id, _, _, _) = side_effect_payload_ref(payload)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    let contract =
        CertifiedSideEffectContract::for_node(runtime_spec.spec(), &contract_node.node_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    validate_current_side_effect_ledger_purpose(
        &contract,
        lifecycle,
        run_id,
        payload,
        &terminal_policies,
    )?;
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
            if let Some(ledger) =
                side_effect_for_attempt(runtime_spec, lifecycle, node, attempt_id)?
            {
                ledger
                    .ledger_state()
                    .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
                let claim = ledger.claim().ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover requires an active claim",
                        node.node_id
                    ))
                })?;
                if &payload.claim_fencing_token == claim.claim_fencing_token() {
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

fn validate_current_side_effect_ledger_purpose(
    contract: &CertifiedSideEffectContract,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    run_id: &RunId,
    payload: &events::KernelEventPayload,
    terminal_policies: &store::SideEffectTerminalPolicies,
) -> Result<()> {
    let purpose = payload
        .side_effect_ledger_ref()
        .map(|side_effect| side_effect.ledger_purpose)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    contract
        .validate_ledger_purpose(purpose)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } = purpose else {
        return Ok(());
    };
    let forward = lifecycle.side_effect(forward_pair_id);
    contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: run_id,
            ledger_purpose: purpose,
            forward_run_id: forward.as_ref().map(|ledger| ledger.run_id()),
            forward_node_id: forward.as_ref().map(|ledger| ledger.intent().node_id()),
            forward_ledger_purpose: forward.as_ref().map(|ledger| ledger.ledger_purpose()),
            forward_terminal: forward
                .as_ref()
                .map(|ledger| {
                    terminal_policies
                        .require(forward_pair_id)
                        .map(|policy| policy.is_terminal_phase(ledger.phase()))
                })
                .transpose()?
                .unwrap_or(false),
        })
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn validate_current_resource_lane_release_payload<S>(
    runtime_spec: &S,
    run_id: &RunId,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    payload: &events::KernelEventPayload,
    expected_authority: Option<events::ResourceLaneReleaseAuthority>,
    verify_node: Option<&spec::NodeSpec>,
) -> Result<bool>
where
    S: CurrentSpecRead + ?Sized,
{
    let Some(resource) = payload.resource_lane_authority_ref() else {
        return Ok(false);
    };
    let Some(release_authority) = resource.release_authority else {
        return Ok(false);
    };
    if let Some(expected) = expected_authority {
        if release_authority != expected {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "resource-lane release authority {} is not valid in this runtime context",
                release_authority.as_str()
            )));
        }
    }
    if resource.ledger.pair_role != events::SideEffectPairRole::Verify {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "resource-lane release for pair {} must use verify role",
            resource.ledger.pair_id
        )));
    }
    let pair = runtime_spec
        .spec()
        .side_effect_verify_pair_for_pair_id(resource.ledger.pair_id)
        .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
    if let Some(node) = verify_node {
        let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} emitted a resource-lane release without verify framework authority",
                node.node_id
            )));
        };
        if verify.pair_id != *resource.ledger.pair_id || pair.verify_node.node_id != node.node_id {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} emitted a resource-lane release outside certified pair {}",
                node.node_id, resource.ledger.pair_id
            )));
        }
    }
    let contract =
        CertifiedSideEffectContract::for_node(runtime_spec.spec(), &pair.submit_node.node_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    validate_current_side_effect_ledger_purpose(
        &contract,
        lifecycle,
        run_id,
        payload,
        &store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?,
    )?;
    Ok(true)
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
