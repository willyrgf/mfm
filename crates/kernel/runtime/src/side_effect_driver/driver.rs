use super::*;

impl SideEffectDriver {
    /// Drives one side-effect protocol step for a prepared runner context.
    pub async fn drive<C>(ctx: ErasedRunCtx<'_>, callbacks: &C) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectDriverCallbacks + ?Sized,
    {
        let view = SideEffectAttemptView::from_erased_context(&ctx)?;
        let action = SideEffectProtocolAction::from_attempt_view(&view)?;
        match action {
            SideEffectProtocolAction::PrepareAndStart => {
                Self::prepare_and_start(&ctx, callbacks).await
            }
            SideEffectProtocolAction::PrepareAndStartClaimed { invocation_epoch } => {
                Self::prepare_and_start_claimed(&ctx, callbacks, &view, invocation_epoch).await
            }
            SideEffectProtocolAction::SubmitOrRecoverSubmission { invocation_epoch }
            | SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission {
                invocation_epoch,
            } => {
                Self::submit_or_recover_submission(&ctx, callbacks, &view, action, invocation_epoch)
                    .await
            }
            SideEffectProtocolAction::CompleteSubmissionBoundary { invocation_epoch } => {
                side_effect_binding(&view, invocation_epoch)?;
                let payloads = RunnerPayloadBuilder::new(&ctx);
                let skip_reason = events::SkipReason {
                    code: events::ErrorCode::new("side_effect_submission_boundary")?,
                    safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
                };
                Ok(ErasedRunnerOutput::new(vec![
                    payloads.cell_skipped(skip_reason)
                ]))
            }
        }
    }

    async fn prepare_and_start<C>(
        ctx: &ErasedRunCtx<'_>,
        callbacks: &C,
    ) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectDriverCallbacks + ?Sized,
    {
        if matches!(
            ctx.node()
                .side_effect
                .as_ref()
                .map(|side_effect| &side_effect.resource_claim),
            Some(spec::ResourceClaimSpec::Exclusive { .. })
        ) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "exclusive side-effect node {} must commit ResourceLaneClaimed before invocation preparation",
                ctx.node().node_id
            )));
        }
        let plan = callbacks.intent_and_idempotency(ctx).await?;
        let side_effect = runtime_side_effect_binding(ctx)?;
        let claim = claim_authority_for(
            ctx.run_id(),
            &ctx.node().node_id,
            ctx.attempt_id(),
            &side_effect,
            1,
            None,
        )?;
        let prepared = callbacks.prepare_invocation(ctx, &plan).await?;
        let builder = SideEffectEvidenceBuilder::new(ctx);
        let evidence = SideEffectPrepareEvidence {
            side_effect,
            claim,
            intent: &plan.intent,
            idempotency: &plan.idempotency,
            idempotency_key: plan.idempotency_key,
            capability_binding: plan.capability_binding,
        };
        if let Some(prepared) = prepared {
            builder.prepare_invocation_and_start(evidence, &prepared)
        } else {
            builder.prepare_and_start(evidence)
        }
    }

    async fn prepare_and_start_claimed<C>(
        ctx: &ErasedRunCtx<'_>,
        callbacks: &C,
        view: &SideEffectAttemptView<'_>,
        invocation_epoch: u32,
    ) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectDriverCallbacks + ?Sized,
    {
        let plan = callbacks.intent_and_idempotency(ctx).await?;
        let side_effect = side_effect_binding(view, invocation_epoch)?;
        let expected = runtime_side_effect_binding(ctx)?;
        if expected.ledger_key != side_effect.ledger_key
            || expected.ledger_purpose != side_effect.ledger_purpose
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} resolved ledger identity that differs from the committed claim",
                ctx.node().node_id
            )));
        }
        let prepared = callbacks.prepare_invocation(ctx, &plan).await?;
        let projection = view
            .projection()
            .ok_or_else(|| missing_driver_projection("claimed side-effect projection"))?;
        let claim = match view.phase() {
            Some(store::SideEffectLedgerPhase::Claimed { claim }) => {
                RuntimeSideEffectClaimAuthority {
                    claim_owner: claim.claim_owner.clone(),
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token.clone(),
                    resource_key: projection.resource_key.clone(),
                }
            }
            _ => return Err(missing_driver_projection("claimed authority")),
        };
        SideEffectEvidenceBuilder::new(ctx).prepare_claimed_invocation_and_start(
            side_effect,
            claim,
            prepared.as_ref(),
        )
    }

    async fn submit_or_recover_submission<C>(
        ctx: &ErasedRunCtx<'_>,
        callbacks: &C,
        view: &SideEffectAttemptView<'_>,
        action: SideEffectProtocolAction,
        invocation_epoch: u32,
    ) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectDriverCallbacks + ?Sized,
    {
        let prepared = match view
            .projection()
            .and_then(|projection| projection.prepared_invocation.as_ref())
        {
            Some(prepared) => Some(
                callbacks
                    .reconstruct_prepared_invocation(ctx, prepared)
                    .await?,
            ),
            None => None,
        };
        let decision = callbacks
            .submit_or_recover_submission(ctx, action, prepared)
            .await?;
        let side_effect = side_effect_binding(view, invocation_epoch)?;
        let builder = SideEffectEvidenceBuilder::new(ctx);
        let start_claim = match action {
            SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission { .. } => {
                match view.phase() {
                    Some(store::SideEffectLedgerPhase::Prepared { claim, .. }) => {
                        Some(RunnerClaimBinding {
                            claim_owner: claim.claim_owner.clone(),
                            claim_generation: claim.claim_generation,
                            claim_fencing_token: claim.claim_fencing_token.clone(),
                        })
                    }
                    _ => return Err(missing_driver_projection("prepared claim")),
                }
            }
            _ => None,
        };
        match decision {
            SideEffectSubmissionDecision::Observed(submission) => builder
                .submission_observed_with_role(
                    side_effect,
                    events::SideEffectPairRole::Submit,
                    start_claim,
                    &submission,
                ),
            SideEffectSubmissionDecision::Unknown(evidence) => {
                builder.submission_unknown(side_effect, start_claim, &evidence)
            }
            SideEffectSubmissionDecision::NotSubmitted(proof) => builder
                .not_submitted_proven_with_role(
                    side_effect,
                    events::SideEffectPairRole::Submit,
                    start_claim,
                    &proof,
                ),
            SideEffectSubmissionDecision::Ambiguous {
                ambiguity_code,
                evidence,
            } => builder.ambiguous(
                side_effect,
                events::SideEffectPairRole::Submit,
                start_claim,
                ambiguity_code,
                &evidence,
            ),
        }
    }
}
