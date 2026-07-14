use super::*;

#[derive(Clone)]
pub(super) struct DriverSideEffectRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl DriverSideEffectRunner {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }

    pub(super) fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.callbacks = self.callbacks.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for DriverSideEffectRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            preclaim_side_effect_resource_lane(
                ctx,
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectDriver::drive(ctx, &self.callbacks).await })
    }
}

#[derive(Clone)]
pub(super) struct DriverSideEffectVerifyRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl DriverSideEffectVerifyRunner {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }
}

impl ErasedNodeRunner for DriverSideEffectVerifyRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectVerifyDriver::drive(ctx, &self.callbacks).await })
    }
}

#[derive(Clone)]
pub(super) struct FailingAfterPreclaimRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl FailingAfterPreclaimRunner {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }
}

impl ErasedNodeRunner for FailingAfterPreclaimRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            preclaim_side_effect_resource_lane(
                ctx,
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::InvalidRunnerOutput(
                "prepare invocation failed after exclusive resource claim".to_owned(),
            ))
        })
    }
}

pub(super) struct PrePreparedSideEffectRunner {
    pub(super) emit_claim: bool,
}

impl ErasedNodeRunner for PrePreparedSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if side_effect_projection_for_attempt(
                ctx.runtime_spec(),
                ctx.run_id(),
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?
            .is_some()
            {
                return Err(RuntimeError::Blocked(
                    "pre-prepared side-effect runner should not resume".to_owned(),
                ));
            }
            let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
            let SideEffectFixtureIntentOutput {
                ledger,
                staged_artifact,
                payload,
            } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
            let mut payloads = vec![payload];
            if self.emit_claim {
                payloads.push(side_effect_claimed(&ctx, ledger, 1, 1));
            }
            Ok(ErasedRunnerOutput::from_parts(
                vec![staged_artifact],
                Vec::new(),
                payloads,
            ))
        })
    }
}

#[derive(Clone)]
pub(super) struct TouchedSetSideEffectVerifyRunner {
    callbacks: TestSideEffectDriverCallbacks,
    receipt: TouchedSetEmission,
    confirmation: TouchedSetEmission,
}

#[derive(Clone, Copy)]
enum TouchedSetEmission {
    None,
    MatchPayloadSchema,
}

impl TouchedSetSideEffectVerifyRunner {
    pub(super) fn with_receipt(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
            receipt: TouchedSetEmission::MatchPayloadSchema,
            confirmation: TouchedSetEmission::None,
        }
    }

    pub(super) fn with_confirmation(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
            receipt: TouchedSetEmission::None,
            confirmation: TouchedSetEmission::MatchPayloadSchema,
        }
    }
}

impl ErasedNodeRunner for TouchedSetSideEffectVerifyRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let mut output = SideEffectVerifyDriver::drive(ctx, &self.callbacks).await?;
            for payload in output.payloads_mut() {
                match payload {
                    RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.receipt,
                            &payload.receipt_schema_id,
                            &payload.receipt_hash,
                            &payload.receipt_artifact_id,
                            &payload.receipt_artifact_evidence_hash,
                        );
                    }
                    RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.confirmation,
                            &payload.confirmation_schema_id,
                            &payload.confirmation_hash,
                            &payload.confirmation_artifact_id,
                            &payload.confirmation_artifact_evidence_hash,
                        );
                    }
                    _ => {}
                }
            }
            Ok(output)
        })
    }
}

pub(super) struct FailActiveSideEffectAfterSagaRunner {
    inner: DriverSideEffectRunner,
    stop_before_invocation_started: bool,
}

impl FailActiveSideEffectAfterSagaRunner {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: false,
        }
    }

    pub(super) fn before_invocation_started(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: true,
        }
    }

    pub(super) fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.inner = self.inner.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for FailActiveSideEffectAfterSagaRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let terminal_policies = runtime_spec_terminal_policies(ctx.runtime_spec());
            let saga = ctx
                .projections()
                .derive_saga_projection(
                    ctx.run_id(),
                    &ctx.runtime_spec().spec().saga,
                    &terminal_policies,
                )
                .expect("saga projection");
            let projected = side_effect_projection_for_attempt(
                ctx.runtime_spec(),
                ctx.run_id(),
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?
            .map(|projection| (projection.ledger_key.clone(), projection.phase.clone()));
            if saga.engagement.is_some() {
                if let Some((ledger, phase)) = &projected {
                    if let Some((invocation_epoch, failure_phase)) =
                        saga_closure_failure_for_phase(phase)
                    {
                        return Ok(ErasedRunnerOutput::new(vec![side_effect_failed(
                            &ctx,
                            ledger.clone(),
                            invocation_epoch,
                            failure_phase,
                            false,
                        )]));
                    }
                }
            }
            if self.stop_before_invocation_started && projected.is_none() {
                return prepared_boundary_side_effect_output(ctx);
            }
            self.inner.run_erased(ctx).await
        })
    }
}

fn prepared_boundary_side_effect_output(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
    let SideEffectFixtureIntentOutput {
        ledger,
        staged_artifact,
        payload,
    } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![staged_artifact],
        Vec::new(),
        vec![
            payload,
            side_effect_claimed(&ctx, ledger.clone(), 1, 1),
            side_effect_prepared(&ctx, ledger, 1, 1),
        ],
    ))
}

fn saga_closure_failure_for_phase(
    phase: &store::SideEffectPhase,
) -> Option<(u32, events::side_effect::FailurePhase)> {
    match phase {
        store::SideEffectPhase::IntentPersisted { invocation_epoch }
        | store::SideEffectPhase::Claimed {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::InvocationPrepared {
            invocation_epoch, ..
        } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::BeforeInvocationStarted,
        )),
        store::SideEffectPhase::NotSubmittedProven { invocation_epoch } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::AfterNotSubmittedProven,
        )),
        _ => None,
    }
}

fn touched_set_for_emission(
    emission: TouchedSetEmission,
    evidence_schema_id: &SchemaId,
    evidence_hash: &ContentDigest,
    evidence_artifact_id: &ArtifactId,
    evidence_artifact_evidence_hash: &ContentDigest,
) -> Option<events::ResourceTouchedSetEvidence> {
    match emission {
        TouchedSetEmission::None => None,
        TouchedSetEmission::MatchPayloadSchema => Some(events::ResourceTouchedSetEvidence {
            namespace: exact_touched_set_resource_namespace(),
            evidence_schema_id: evidence_schema_id.clone(),
            evidence_hash: evidence_hash.clone(),
            evidence_artifact_id: evidence_artifact_id.clone(),
            evidence_artifact_evidence_hash: evidence_artifact_evidence_hash.clone(),
        }),
    }
}

pub(super) struct ForwardEmitsRemediationPurposeRunner;

impl ErasedNodeRunner for ForwardEmitsRemediationPurposeRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            side_effect_intent_with_purpose(
                ctx,
                events::SideEffectLedgerPurpose::Remediation {
                    forward_pair_id: synthetic_side_effect_pair_id(0x84),
                },
            )
        })
    }
}

pub(super) struct RemediationEmitsForwardPurposeRunner {
    inner: DriverSideEffectRunner,
}

impl RemediationEmitsForwardPurposeRunner {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for RemediationEmitsForwardPurposeRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if ctx
                .runtime_spec()
                .forward_node_for_remediation(&ctx.node().node_id)
                .is_some()
            {
                side_effect_intent_with_purpose(ctx, events::SideEffectLedgerPurpose::Forward)
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

fn side_effect_intent_with_purpose(
    ctx: ErasedRunCtx<'_>,
    ledger_purpose: events::SideEffectLedgerPurpose,
) -> Result<ErasedRunnerOutput> {
    let SideEffectFixtureIntentOutput {
        staged_artifact,
        payload,
        ..
    } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![staged_artifact],
        Vec::new(),
        vec![payload],
    ))
}

pub(super) struct PrematureSideEffectOutputRunner {
    pub(super) output_artifact: ArtifactId,
    pub(super) output_digest: ContentDigest,
}

impl ErasedNodeRunner for PrematureSideEffectOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let artifact = state_output_artifact(
                ctx.node(),
                ctx.descriptor(),
                self.output_artifact.clone(),
                self.output_digest.clone(),
            );
            let staged_artifact = staged_attempt_artifact(&ctx, artifact.clone())?;
            Ok(ErasedRunnerOutput::from_parts(
                vec![staged_artifact],
                Vec::new(),
                terminal_payloads(&ctx, &artifact),
            ))
        })
    }
}
