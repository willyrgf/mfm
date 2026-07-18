use super::*;

impl SideEffectDriver {
    /// Drives one side-effect protocol step for a prepared runner context.
    pub async fn drive<A>(ctx: ErasedRunCtx<'_>, adapter: &A) -> Result<ErasedRunnerOutput>
    where
        A: SideEffectAdapter + ?Sized,
    {
        let view = SideEffectAttemptView::from_erased_context(&ctx)?;
        match SideEffectStep::from_attempt_view(&view)? {
            SideEffectStep::Claim => Self::claim(&ctx, adapter).await,
            SideEffectStep::Prepare { invocation_epoch } => {
                Self::prepare(&ctx, adapter, &view, invocation_epoch).await
            }
            SideEffectStep::Start { invocation_epoch } => {
                Self::start(&ctx, &view, invocation_epoch)
            }
            SideEffectStep::Submit { invocation_epoch } => {
                Self::submit(&ctx, adapter, &view, invocation_epoch).await
            }
            SideEffectStep::CompleteSubmissionBoundary { invocation_epoch } => {
                side_effect_binding(&view, invocation_epoch)?;
                let skip_reason = events::SkipReason {
                    code: events::ErrorCode::new("side_effect_submission_boundary")?,
                    safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
                };
                Ok(ErasedRunnerOutput::new(vec![RunnerPayloadBuilder::new(
                    &ctx,
                )
                .cell_skipped(skip_reason)]))
            }
        }
    }

    async fn claim<A>(ctx: &ErasedRunCtx<'_>, adapter: &A) -> Result<ErasedRunnerOutput>
    where
        A: SideEffectAdapter + ?Sized,
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
        let plan = authored_plan(ctx, adapter, ctx.node(), ctx.inputs()).await?;
        let side_effect = runtime_side_effect_binding(ctx)?;
        let claim = claim_authority_for(
            ctx.run_id(),
            &ctx.node().node_id,
            ctx.attempt_id(),
            &side_effect,
            1,
            None,
        )?;
        SideEffectEvidenceBuilder::new(ctx).claim_intent(SideEffectClaimEvidence {
            side_effect,
            claim,
            intent: &plan.intent,
            idempotency: &plan.idempotency,
            capability_binding: plan.capability_binding,
        })
    }

    async fn prepare<A>(
        ctx: &ErasedRunCtx<'_>,
        adapter: &A,
        view: &SideEffectAttemptView<'_>,
        invocation_epoch: u32,
    ) -> Result<ErasedRunnerOutput>
    where
        A: SideEffectAdapter + ?Sized,
    {
        let plan = authored_plan(ctx, adapter, ctx.node(), ctx.inputs()).await?;
        verify_authored_plan(
            view.projection()
                .ok_or_else(|| missing_driver_projection("side-effect projection"))?,
            &plan,
        )?;
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
        let prepared = adapter
            .prepare(ctx, &plan.intent, &plan.idempotency)
            .await?;
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
            &prepared,
        )
    }

    fn start(
        ctx: &ErasedRunCtx<'_>,
        view: &SideEffectAttemptView<'_>,
        invocation_epoch: u32,
    ) -> Result<ErasedRunnerOutput> {
        let claim = match view.phase() {
            Some(store::SideEffectLedgerPhase::Prepared { claim, .. }) => RunnerClaimBinding {
                claim_owner: claim.claim_owner.clone(),
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token.clone(),
            },
            _ => return Err(missing_driver_projection("prepared claim")),
        };
        Ok(SideEffectEvidenceBuilder::new(ctx)
            .start_prepared(side_effect_binding(view, invocation_epoch)?, claim))
    }

    async fn submit<A>(
        ctx: &ErasedRunCtx<'_>,
        adapter: &A,
        view: &SideEffectAttemptView<'_>,
        invocation_epoch: u32,
    ) -> Result<ErasedRunnerOutput>
    where
        A: SideEffectAdapter + ?Sized,
    {
        let plan = authored_plan(ctx, adapter, ctx.node(), ctx.inputs()).await?;
        verify_authored_plan(
            view.projection()
                .ok_or_else(|| missing_driver_projection("side-effect projection"))?,
            &plan,
        )?;
        let prepared = view
            .projection()
            .and_then(|projection| projection.prepared_invocation.as_ref())
            .ok_or_else(|| missing_driver_projection("prepared invocation"))?;
        let prepared = adapter.load_prepared(ctx, ctx.node(), prepared).await?;
        let decision = adapter.submit_prepared(ctx, prepared).await?;
        let side_effect = side_effect_binding(view, invocation_epoch)?;
        let builder = SideEffectEvidenceBuilder::new(ctx);
        match decision {
            SideEffectSubmissionDecision::Observed(submission) => builder
                .submission_observed_with_role(
                    side_effect,
                    events::SideEffectPairRole::Submit,
                    None,
                    &submission,
                ),
            SideEffectSubmissionDecision::Unknown(evidence) => {
                builder.submission_unknown(side_effect, None, &evidence)
            }
            SideEffectSubmissionDecision::NotSubmitted(proof) => builder
                .not_submitted_proven_with_role(
                    side_effect,
                    events::SideEffectPairRole::Submit,
                    None,
                    &proof,
                ),
            SideEffectSubmissionDecision::Ambiguous {
                ambiguity_code,
                evidence,
            } => builder.ambiguous(
                side_effect,
                events::SideEffectPairRole::Submit,
                None,
                ambiguity_code,
                &evidence,
            ),
        }
    }
}

pub(crate) async fn authored_plan<A>(
    ctx: &ErasedRunCtx<'_>,
    adapter: &A,
    submit_node: &spec::NodeSpec,
    submit_inputs: &MaterializedInputs,
) -> Result<AuthoredSideEffect<A::Intent, A::Idempotency>>
where
    A: SideEffectAdapter + ?Sized,
{
    let (intent, idempotency) = adapter
        .authored_intent(ctx, submit_node, submit_inputs)
        .await?
        .into_parts();
    AuthoredSideEffect::new(intent, idempotency, adapter.capability_binding()?)
}

pub(crate) fn verify_authored_plan<Intent, Idempotency>(
    projection: &store::SideEffectProjection,
    plan: &AuthoredSideEffect<Intent, Idempotency>,
) -> Result<()>
where
    Intent: MfmValue,
    Idempotency: MfmValue,
{
    let intent = &projection.intent;
    let intent_schema_id = Intent::schema_id()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let idempotency_schema_id = Idempotency::schema_id()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let intent_hash = canonical_mfm_value(&plan.intent)?.content_digest();
    let idempotency_hash = canonical_mfm_value(&plan.idempotency)?.content_digest();
    let binding = &plan.capability_binding;
    if intent.intent_schema_id != intent_schema_id
        || intent.intent_hash != intent_hash
        || intent.idempotency_input_schema_id != idempotency_schema_id
        || intent.idempotency_input_hash != idempotency_hash
        || intent.idempotency_key != plan.idempotency_key
        || intent.capability_kind != binding.capability_kind
        || intent.capability_version != binding.capability_version
        || intent.adapter_kind != binding.adapter_kind
        || intent.adapter_version != binding.adapter_version
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} recomputed authored authority that differs from retained intent",
            intent.node_id
        )));
    }
    Ok(())
}
