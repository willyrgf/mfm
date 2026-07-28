use super::*;

pub(super) fn runner_output_preconditions<S>(
    runtime_spec: &S,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    payloads: &[events::KernelEventPayload],
    require_existing_attempt: bool,
) -> Result<store::CommitPreconditions>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        required_public_output_absent: matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ),
        ..store::CommitPreconditions::default()
    };
    if require_existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
    } else {
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
    }
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ) {
        let completion = run_completion_evidence(runtime_spec, lifecycle)?;
        let retention_manifest = current_retention_manifest(lifecycle)?;
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "public_output:{}",
                completion.public_output_schema_id
            ))?);
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "retention:{}:manifest:{}",
                run_id,
                retention_manifest.sequence()
            ))?);
    }
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    ) {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }
    if payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::FactRecorded(_)))
    {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }
    if node.side_effect.is_some() || side_effect_verify_spec(node).is_some() {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }

    if let Some(verify) = side_effect_verify_spec(node) {
        add_side_effect_verify_preconditions(
            runtime_spec,
            lifecycle,
            node,
            verify,
            payloads,
            &mut preconditions,
        )?;
        return Ok(preconditions);
    }

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let terminal_side_effect_required_state = payloads.iter().find_map(|payload| match payload {
        events::KernelEventPayload::CellSkipped(_) => {
            Some(store::RequiredSideEffectState::SubmissionResult)
        }
        events::KernelEventPayload::CellProduced(_) => {
            Some(store::RequiredSideEffectState::ConfirmationObserved)
        }
        _ => None,
    });
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.pair_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.pair_id) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            pair_id: payload.pair_id.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            _ => {}
        }
    }

    if let Some(required) = terminal_side_effect_required_state {
        let ledger = side_effect_for_attempt(runtime_spec, lifecycle, node, attempt_id)?
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempted output without ledger evidence",
                    node.node_id
                ))
            })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                pair_id: ledger.pair_id().clone(),
                required,
            });
    }

    Ok(preconditions)
}

fn add_side_effect_verify_preconditions<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    node: &spec::NodeSpec,
    verify: &spec::SideEffectVerifyNodeSpec,
    payloads: &[events::KernelEventPayload],
    preconditions: &mut store::CommitPreconditions,
) -> Result<()>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    let ledger = lifecycle.side_effect(&verify.pair_id).ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} has no ledger projection for pair {}",
            node.node_id, verify.pair_id
        ))
    })?;
    let terminal_required = side_effect_verify_terminal_required_state(runtime_spec, verify)?;
    for payload in payloads {
        let required = match payload {
            events::KernelEventPayload::SideEffectReceiptObserved(_) => {
                Some(store::RequiredSideEffectState::SubmissionResult)
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(_) => {
                Some(store::RequiredSideEffectState::ReceiptObserved)
            }
            events::KernelEventPayload::SideEffectFailed(_) => {
                Some(store::RequiredSideEffectState::SubmissionResult)
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => Some(terminal_required),
            _ => None,
        };
        if let Some(required) = required {
            push_side_effect_precondition(preconditions, ledger.pair_id().clone(), required);
        }
    }
    Ok(())
}

pub(super) fn side_effect_verify_terminal_required_state<S>(
    runtime_spec: &S,
    verify: &spec::SideEffectVerifyNodeSpec,
) -> Result<store::RequiredSideEffectState>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    let pair = runtime_spec
        .spec()
        .side_effect_verify_pair_for_pair_id(&verify.pair_id)
        .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
    match &pair.submit_contract.verification {
        spec::SideEffectVerificationSpec::Receipt => {
            Ok(store::RequiredSideEffectState::ReceiptObserved)
        }
        spec::SideEffectVerificationSpec::Finalized { .. } => {
            Ok(store::RequiredSideEffectState::ConfirmationObserved)
        }
    }
}

pub(super) fn certified_run_authority<S>(
    runtime_spec: &S,
    run_id: &RunId,
) -> Result<store::CertifiedRunStoreAuthority>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    Ok(store::CertifiedRunStoreAuthority::from_spec(
        run_id.clone(),
        runtime_spec.spec(),
    )?)
}

fn push_side_effect_precondition(
    preconditions: &mut store::CommitPreconditions,
    pair_id: mfm_ids::SideEffectPairId,
    required: store::RequiredSideEffectState,
) {
    if preconditions
        .required_side_effect_states
        .iter()
        .any(|existing| existing.pair_id == pair_id && existing.required == required)
    {
        return;
    }
    preconditions
        .required_side_effect_states
        .push(store::SideEffectStatePrecondition { pair_id, required });
}

pub(super) fn side_effect_verify_spec(
    node: &spec::NodeSpec,
) -> Option<&spec::SideEffectVerifyNodeSpec> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => Some(verify),
        _ => None,
    }
}

pub(super) fn attempt_commit_preconditions<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    existing_attempt: Option<&AttemptId>,
) -> Result<store::CommitPreconditions>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        ..store::CommitPreconditions::default()
    };
    preconditions
        .required_cell_states
        .extend(node_cell_preconditions(runtime_spec, node)?);
    if let Some(attempt_id) = existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
    }
    Ok(preconditions)
}

fn node_cell_preconditions<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
) -> Result<Vec<store::CellStatePrecondition>>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    let mut preconditions = Vec::new();
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        let cell = runtime_spec.cell(&cell_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "validated input binding references missing cell {cell_id}"
            ))
        })?;
        if matches!(cell.producer, spec::CellProducer::Node(_)) {
            preconditions.push(store::CellStatePrecondition {
                cell_id,
                required: store::RequiredCellState::Terminal,
            });
        }
    }
    Ok(preconditions)
}
