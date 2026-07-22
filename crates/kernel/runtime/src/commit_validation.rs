use super::*;
use std::collections::BTreeSet;

pub(crate) fn runner_payloads_with_derived_lifecycle(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    read_facts: Vec<events::FactRecorded>,
    runner_payloads: Vec<RunnerEventPayload>,
) -> Result<Vec<events::KernelEventPayload>> {
    let mut payloads = read_facts
        .into_iter()
        .map(events::KernelEventPayload::FactRecorded)
        .chain(
            runner_payloads
                .into_iter()
                .map(events::KernelEventPayload::from),
        )
        .collect::<Vec<_>>();
    let mut terminal_cell = false;
    let mut failure: Option<(bool, events::MfmErrorInfo)> = None;
    for payload in &payloads {
        match payload {
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_) => {
                terminal_cell = true;
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                if failure
                    .replace((payload.retryable, payload.error.clone()))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                if failure
                    .replace((false, side_effect_ambiguity_error(payload)?))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload)
                if failure
                    .replace((payload.error.retryable, payload.error.clone()))
                    .is_some() =>
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned multiple failure payloads",
                    node.node_id
                )));
            }
            _ => {}
        }
    }

    if let Some((retryable, error)) = failure {
        payloads.push(events::KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                retryable,
                error,
            },
        ));
    } else if terminal_cell {
        payloads.push(events::KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            },
        ));
    }

    Ok(payloads)
}

fn side_effect_ambiguity_error(
    payload: &events::side_effect::Ambiguous,
) -> Result<events::MfmErrorInfo> {
    Ok(events::MfmErrorInfo::new(
        events::ErrorCode::new("side_effect_ambiguous")?,
        events::ErrorCategory::SideEffect,
        false,
        format!(
            "side-effect outcome is ambiguous: {}",
            payload.ambiguity_code
        ),
    )?)
}

pub(super) struct RunnerOutputValidation<'a> {
    pub(super) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(super) run_id: &'a RunId,
    pub(super) node: &'a spec::NodeSpec,
    pub(super) attempt_id: &'a AttemptId,
    pub(super) caps: &'a CertifiedRuntimeCapabilities,
    pub(super) projections: &'a store::ProjectionSnapshot,
    pub(super) payloads: &'a [events::KernelEventPayload],
}

pub(super) fn validate_runner_output(input: RunnerOutputValidation<'_>) -> Result<()> {
    let RunnerOutputValidation {
        runtime_spec,
        run_id,
        node,
        attempt_id,
        caps,
        projections,
        payloads,
    } = input;
    if payloads.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let mut completed = false;
    let mut failed = false;
    let mut terminal_cell = false;
    let mut public_output_produced = false;
    let mut public_output_failed = false;
    let mut side_effect_payload = false;
    let mut side_effect_terminal_failure = false;
    let mut attempt_failure_retryable = None;
    let mut side_effect_terminal_failure_retryable = None;
    let mut fact_keys = BTreeSet::new();
    let mut fact_count = 0usize;
    let mut fact_phase_closed = false;
    for payload in payloads {
        if payload_spec_hash(payload) != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} returned payload with mismatched spec hash",
                node.node_id
            )));
        }
        match payload {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                fact_phase_closed = true;
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} completed output cell {} instead of certified {}",
                        node.node_id, payload.output_cell_id, node.output_cell
                    )));
                }
                completed = true;
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                fact_phase_closed = true;
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if failed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
                attempt_failure_retryable = Some(payload.retryable);
                failed = true;
            }
            events::KernelEventPayload::StateAttemptInterrupted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned recovery-owned StateAttemptInterrupted",
                    node.node_id
                )));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                fact_phase_closed = true;
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.context != payload.context
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced cell metadata outside certified spec",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                fact_phase_closed = true;
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.context != payload.context
                    || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped a cell outside certified skip policy",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let fact_key = payload.claim.subject().fact_key();
                if fact_phase_closed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} returned fact {} after its settlement cell or terminal payload",
                        node.node_id, fact_key
                    )));
                }
                if node.fact_descriptor_allowlist.len() != 1
                    || node.fact_descriptor_allowlist[0].descriptor_hash
                        != *payload.claim.fact_descriptor_hash()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} returned fact {} outside its one certified descriptor",
                        node.node_id, fact_key
                    )));
                }
                if !fact_keys.insert(fact_key.clone()) {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} returned duplicate fact key {}",
                        node.node_id, fact_key
                    )));
                }
                fact_count += 1;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned artifact reference payload for {}",
                    node.node_id, payload.artifact_ref.artifact_id
                )));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output(runtime_spec, node, payload)?;
                public_output_produced = true;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )?;
                public_output_failed = true;
            }
            events::KernelEventPayload::RunAdmitted(_)
            | events::KernelEventPayload::ManualResolutionRecorded(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::ResourceLaneClaimed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::ResourceLaneClaimIntent(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                validate_runner_side_effect_payload(
                    runtime_spec,
                    run_id,
                    node,
                    attempt_id,
                    caps,
                    projections,
                    payload,
                )?;
                side_effect_payload = true;
                match payload {
                    events::KernelEventPayload::SideEffectFailed(payload) => {
                        side_effect_terminal_failure = true;
                        side_effect_terminal_failure_retryable = Some(payload.retryable);
                    }
                    events::KernelEventPayload::SideEffectAmbiguous(_) => {
                        side_effect_terminal_failure = true;
                        side_effect_terminal_failure_retryable = Some(false);
                    }
                    _ => {}
                }
            }
        }
    }

    let descriptor = runtime_spec.state_descriptor_for_node(node)?;
    if fact_count > 0
        && (descriptor.effect_class != "read_external"
            || descriptor.emitted_fact_descriptors.len() != 1)
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} emitted facts without one certified fact-producing external-read contract",
            node.node_id
        )));
    }
    if descriptor.effect_class == "read_external"
        && !descriptor.emitted_fact_descriptors.is_empty()
        && fact_count == 0
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact-producing external-read node {} returned an empty fact batch",
            node.node_id
        )));
    }
    if side_effect_verify_spec(node).is_some() {
        return validate_side_effect_verify_runner_output(SideEffectVerifyRunnerOutputValidation {
            runtime_spec,
            run_id,
            node,
            projections,
            completed,
            failed,
            terminal_cell,
            public_output_produced,
            public_output_failed,
            side_effect_payload,
            side_effect_terminal_failure,
            attempt_failure_retryable,
            side_effect_terminal_failure_retryable,
            payloads,
        });
    }

    if node.side_effect.is_some() {
        validate_resume_output(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
            payloads,
        )?;
        if failed {
            if !side_effect_terminal_failure {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned StateAttemptFailed without terminal side-effect evidence",
                    node.node_id
                )));
            }
            if side_effect_terminal_failure_retryable != attempt_failure_retryable {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned inconsistent failure retryability",
                    node.node_id
                )));
            }
            if completed || terminal_cell || public_output_produced {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} mixed side-effect failure with successful terminal evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if side_effect_terminal_failure {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} returned terminal side-effect evidence without StateAttemptFailed",
                node.node_id
            )));
        }
        if side_effect_payload {
            if completed || terminal_cell || public_output_produced || public_output_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} mixed ledger phase events with terminal output evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if !completed || !terminal_cell {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} output commit must pair StateAttemptCompleted with terminal cell evidence",
                node.node_id
            )));
        }
        let terminal_skipped = payloads
            .iter()
            .any(|payload| matches!(payload, events::KernelEventPayload::CellSkipped(_)));
        validate_terminal_batch_evidence(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
            terminal_skipped,
        )
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if public_output_produced && !completed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} projected public output without completing its certified output cell",
                node.node_id
            )));
        }
        return Ok(());
    }
    if failed && (completed || terminal_cell || public_output_produced) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} mixed failure with successful terminal evidence",
            node.node_id
        )));
    }
    if public_output_failed && !failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned public-output failure without StateAttemptFailed",
            node.node_id
        )));
    }
    if !failed && (!completed || !terminal_cell) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_produced && !completed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} projected public output without completing its certified output cell",
            node.node_id
        )));
    }
    Ok(())
}

struct SideEffectVerifyRunnerOutputValidation<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    projections: &'a store::ProjectionSnapshot,
    completed: bool,
    failed: bool,
    terminal_cell: bool,
    public_output_produced: bool,
    public_output_failed: bool,
    side_effect_payload: bool,
    side_effect_terminal_failure: bool,
    attempt_failure_retryable: Option<bool>,
    side_effect_terminal_failure_retryable: Option<bool>,
    payloads: &'a [events::KernelEventPayload],
}

fn validate_side_effect_verify_runner_output(
    input: SideEffectVerifyRunnerOutputValidation<'_>,
) -> Result<()> {
    let SideEffectVerifyRunnerOutputValidation {
        runtime_spec,
        run_id,
        node,
        projections,
        completed,
        failed,
        terminal_cell,
        public_output_produced,
        public_output_failed,
        side_effect_payload,
        side_effect_terminal_failure,
        attempt_failure_retryable,
        side_effect_terminal_failure_retryable,
        payloads,
    } = input;
    if failed {
        if !side_effect_terminal_failure {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned StateAttemptFailed without terminal side-effect evidence",
                node.node_id
            )));
        }
        if side_effect_terminal_failure_retryable != attempt_failure_retryable {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned inconsistent failure retryability",
                node.node_id
            )));
        }
        if completed || terminal_cell || public_output_produced {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} mixed side-effect failure with successful terminal evidence",
                node.node_id
            )));
        }
        return Ok(());
    }
    if side_effect_terminal_failure {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} returned terminal side-effect evidence without StateAttemptFailed",
            node.node_id
        )));
    }
    if side_effect_payload && !terminal_cell && !completed {
        let has_resource_lane_release = payloads.iter().any(|payload| {
            matches!(
                payload,
                events::KernelEventPayload::ResourceLaneReleaseIntent(_)
            )
        });
        let has_side_effect_terminal_disposition = payloads
            .iter()
            .any(events::KernelEventPayload::is_side_effect_terminal_disposition);
        if has_resource_lane_release && !has_side_effect_terminal_disposition {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned a resource-lane release without terminal evidence",
                node.node_id
            )));
        }
        return Ok(());
    }
    if !completed || !terminal_cell {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} returned public-output failure",
            node.node_id
        )));
    }
    if side_effect_payload
        && payloads.iter().any(|payload| {
            payload.side_effect_ledger_ref().is_some()
                && !matches!(
                    payload,
                    events::KernelEventPayload::ResourceLaneReleaseIntent(_)
                )
        })
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} mixed ledger phase events with terminal output evidence",
            node.node_id
        )));
    }
    validate_side_effect_verify_terminal_evidence(runtime_spec, run_id, node, projections)
}

fn validate_side_effect_verify_terminal_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let Some(verify) = side_effect_verify_spec(node) else {
        return Ok(());
    };
    let projection = projections
        .side_effect_for_pair(run_id, &verify.pair_id)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} has no ledger projection for pair {}",
                node.node_id, verify.pair_id
            ))
        })?;
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let required = side_effect_verify_terminal_required_state(runtime_spec, verify)?;
    let satisfied = match required {
        store::RequiredSideEffectState::ReceiptObserved => matches!(
            state.phase(),
            store::SideEffectLedgerPhase::ReceiptObserved { .. }
                | store::SideEffectLedgerPhase::Confirmed { .. }
        ),
        store::RequiredSideEffectState::ConfirmationObserved => {
            matches!(
                state.phase(),
                store::SideEffectLedgerPhase::Confirmed { .. }
            )
        }
        _ => false,
    };
    if satisfied {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} produced output before certified terminal evidence",
            node.node_id
        )))
    }
}

pub(super) fn runtime_fact_error(error: mfm_facts::FactError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}
