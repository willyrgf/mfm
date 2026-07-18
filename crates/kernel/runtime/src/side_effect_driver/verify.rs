use super::*;

impl SideEffectVerifyDriver {
    /// Drives one verification step for a certified side-effect verify framework node.
    pub async fn drive<C>(ctx: ErasedRunCtx<'_>, callbacks: &C) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectAdapter + ?Sized,
    {
        let verify = match &ctx.node().framework {
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => verify,
            _ => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} is not a side-effect verify framework node",
                    ctx.node().node_id
                )));
            }
        };
        let pair = ctx
            .runtime_spec()
            .spec()
            .side_effect_verify_pair_for_verify_node(&ctx.node().node_id)
            .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        let submit_node = pair.submit_node;
        let submit_contract = pair.submit_contract;
        let projection = ctx
            .projections()
            .side_effect_for_pair(ctx.run_id(), &verify.pair_id)
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect verify node {} has no ledger projection for pair {}",
                    ctx.node().node_id,
                    verify.pair_id
                ))
            })?;
        if projection.intent.node_id != submit_node.node_id {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect pair {} ledger belongs to submit node {} instead of certified {}",
                verify.pair_id, projection.intent.node_id, submit_node.node_id
            )));
        }
        let state = projection
            .ledger_state()
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
        let submit_inputs = ctx.materialize_node_inputs(submit_node)?;
        let plan = authored_plan(&ctx, callbacks, submit_node, &submit_inputs).await?;
        verify_authored_plan(projection, &plan)?;
        let prepared = projection
            .prepared_invocation
            .as_ref()
            .ok_or_else(|| missing_driver_projection("prepared invocation"))?;
        let phase = state.phase();
        let side_effect = RunnerSideEffectBinding {
            ledger_key: projection.ledger_key.clone(),
            ledger_purpose: projection.ledger_purpose.clone(),
            pair_id: verify.pair_id.clone(),
            invocation_epoch: phase.invocation_epoch(),
        };
        match phase {
            store::SideEffectLedgerPhase::SubmissionKnown {
                status: store::SideEffectSubmissionState::Observed { submission },
                ..
            } => {
                let receipt = callbacks
                    .observe_receipt(&ctx, submit_node, &submit_inputs, prepared, submission)
                    .await?;
                let builder = SideEffectEvidenceBuilder::new(&ctx);
                match &submit_contract.verification {
                    spec::SideEffectVerificationSpec::Receipt => builder.receipt_observed_terminal(
                        side_effect,
                        &receipt.evidence,
                        receipt.replay,
                    ),
                    spec::SideEffectVerificationSpec::Finalized { .. } => {
                        builder.receipt_observed(side_effect, &receipt.evidence, receipt.replay)
                    }
                }
            }
            store::SideEffectLedgerPhase::SubmissionKnown {
                status: store::SideEffectSubmissionState::NotSubmitted,
                ..
            } => {
                let payloads = RunnerPayloadBuilder::new(&ctx);
                let mut output = RunnerOutputBuilder::new(&ctx);
                if let Some(release) = SideEffectEvidenceBuilder::new(&ctx)
                    .resource_lane_released_payload(side_effect.clone(), "side_effect.failed")?
                {
                    output.payload(release);
                }
                let error = events::MfmErrorInfo::new(
                    events::ErrorCode::new("side_effect_verification_failed")?,
                    events::ErrorCategory::SideEffect,
                    false,
                    "side-effect invocation was proven not submitted",
                )?;
                output.payload(payloads.side_effect_failed(
                    side_effect,
                    events::SideEffectPairRole::Verify,
                    side_effect::FailurePhase::AfterNotSubmittedProven,
                    false,
                    error,
                ));
                Ok(output.finish())
            }
            store::SideEffectLedgerPhase::SubmissionKnown {
                status: store::SideEffectSubmissionState::Unknown,
                ..
            } => {
                let prepared = callbacks.load_prepared(&ctx, submit_node, prepared).await?;
                let decision = callbacks
                    .recover_unknown(&ctx, submit_node, &submit_inputs, prepared)
                    .await?;
                let builder = SideEffectEvidenceBuilder::new(&ctx);
                match decision {
                    SideEffectUnknownSubmissionDecision::Observed(submission) => builder
                        .submission_observed_with_role(
                            side_effect,
                            events::SideEffectPairRole::Verify,
                            None,
                            &submission,
                        ),
                    SideEffectUnknownSubmissionDecision::StillUnknown => {
                        Err(RuntimeError::Blocked(format!(
                            "side-effect verify node {} still cannot resolve unknown submission for pair {}",
                            ctx.node().node_id,
                            verify.pair_id
                        )))
                    }
                    SideEffectUnknownSubmissionDecision::NotSubmitted(proof) => builder
                        .not_submitted_proven_with_role(
                            side_effect,
                            events::SideEffectPairRole::Verify,
                            None,
                            &proof,
                        ),
                    SideEffectUnknownSubmissionDecision::Ambiguous {
                        ambiguity_code,
                        evidence,
                    } => builder.ambiguous(
                        side_effect,
                        events::SideEffectPairRole::Verify,
                        None,
                        ambiguity_code,
                        &evidence,
                    ),
                }
            }
            store::SideEffectLedgerPhase::ReceiptObserved {
                submission,
                receipt,
                ..
            } => match &submit_contract.verification {
                spec::SideEffectVerificationSpec::Receipt => {
                    let output = callbacks
                        .output_from_receipt(&ctx, &submit_inputs, prepared, submission, receipt)
                        .await?;
                    ErasedRunnerOutput::state_output(&ctx, &output)
                }
                spec::SideEffectVerificationSpec::Finalized { .. } => {
                    let confirmation = callbacks
                        .observe_confirmation(
                            &ctx,
                            submit_node,
                            &submit_inputs,
                            prepared,
                            submission,
                            receipt,
                        )
                        .await?;
                    SideEffectEvidenceBuilder::new(&ctx).confirmation_observed(
                        side_effect,
                        &confirmation.evidence,
                        confirmation.replay,
                    )
                }
            },
            store::SideEffectLedgerPhase::Confirmed {
                submission,
                receipt,
                confirmation,
                ..
            } => match &submit_contract.verification {
                spec::SideEffectVerificationSpec::Receipt => {
                    let output = callbacks
                        .output_from_receipt(&ctx, &submit_inputs, prepared, submission, receipt)
                        .await?;
                    ErasedRunnerOutput::state_output(&ctx, &output)
                }
                spec::SideEffectVerificationSpec::Finalized { .. } => {
                    let output = callbacks
                        .output_from_confirmation(
                            &ctx,
                            &submit_inputs,
                            prepared,
                            submission,
                            receipt,
                            confirmation,
                        )
                        .await?;
                    ErasedRunnerOutput::state_output(&ctx, &output)
                }
            },
            store::SideEffectLedgerPhase::IntentPersisted { .. }
            | store::SideEffectLedgerPhase::Claimed { .. }
            | store::SideEffectLedgerPhase::Prepared { .. }
            | store::SideEffectLedgerPhase::Started { .. }
            | store::SideEffectLedgerPhase::Ambiguous { .. }
            | store::SideEffectLedgerPhase::Failed { .. } => {
                Err(unsupported_side_effect_phase(phase))
            }
        }
    }
}
