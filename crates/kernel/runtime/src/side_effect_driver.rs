use std::future::Future;
use std::pin::Pin;

use mfm_events::v1::{self as events, side_effect};
use mfm_ids::ContentDigest;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::Serialize;

use crate::runner_kit::{
    RunnerClaimBinding, RunnerPreparedInvocationBinding, RunnerSideEffectBinding,
};
use crate::{
    ErasedRunCtx, ErasedRunnerOutput, Result, RunnerArtifactBuilder, RunnerCapabilityBinding,
    RunnerJsonArtifact, RunnerOutputBuilder, RunnerPayloadBuilder, RuntimeError,
    SideEffectAttemptView,
};

/// Boxed future returned by side-effect driver callbacks.
pub type SideEffectDriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Generic side-effect protocol action selected from a verified attempt view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffectProtocolAction {
    /// No ledger evidence exists yet; persist intent and claim the first invocation epoch.
    PrepareAndStart,
    /// A side-effect claim exists and can now prepare/start under its held lane.
    PrepareAndStartClaimed {
        /// Invocation epoch to prepare and start.
        invocation_epoch: u32,
    },
    /// Invocation crossed the external uncertainty boundary; submit or recover submission status.
    SubmitOrRecoverSubmission {
        /// Invocation epoch to submit or recover.
        invocation_epoch: u32,
    },
    /// Invocation was prepared but not marked started; start it, then submit or recover status.
    StartPreparedAndSubmitOrRecoverSubmission {
        /// Invocation epoch to start and submit or recover.
        invocation_epoch: u32,
    },
    /// The previous invocation was proven not submitted; claim the next epoch.
    RetryAfterNotSubmitted {
        /// Invocation epoch to claim and start.
        invocation_epoch: u32,
        /// New claim generation.
        claim_generation: u32,
    },
    /// Submission evidence exists; read receipt evidence.
    ReadReceipt {
        /// Invocation epoch whose submission was observed.
        invocation_epoch: u32,
    },
    /// Receipt evidence exists; build confirmation evidence.
    BuildConfirmation {
        /// Invocation epoch whose receipt was observed.
        invocation_epoch: u32,
    },
    /// Confirmation evidence exists; map it to the state output.
    MapConfirmationToOutput {
        /// Invocation epoch whose confirmation was observed.
        invocation_epoch: u32,
    },
    /// Ambiguity is terminal for this attempt; the driver should not emit new evidence.
    IdleAmbiguous {
        /// Invocation epoch that became ambiguous.
        invocation_epoch: u32,
    },
}

impl SideEffectProtocolAction {
    /// Selects the protocol action for a verified side-effect attempt view.
    pub fn from_attempt_view(view: &SideEffectAttemptView<'_>) -> Result<Self> {
        let Some(phase) = view.phase() else {
            return Ok(Self::PrepareAndStart);
        };
        match phase {
            store::SideEffectLedgerPhase::Prepared { claim, .. } => {
                Ok(Self::StartPreparedAndSubmitOrRecoverSubmission {
                    invocation_epoch: claim.invocation_epoch,
                })
            }
            store::SideEffectLedgerPhase::Claimed { claim } => Ok(Self::PrepareAndStartClaimed {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::Started { claim, .. }
            | store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::Unknown,
            } => Ok(Self::SubmitOrRecoverSubmission {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::NotSubmitted,
            } => Ok(Self::RetryAfterNotSubmitted {
                invocation_epoch: claim.invocation_epoch + 1,
                claim_generation: claim.claim_generation + 1,
            }),
            store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::Observed { .. },
            } => Ok(Self::ReadReceipt {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::ReceiptObserved { claim, .. } => {
                Ok(Self::BuildConfirmation {
                    invocation_epoch: claim.invocation_epoch,
                })
            }
            store::SideEffectLedgerPhase::Confirmed { claim, .. } => {
                Ok(Self::MapConfirmationToOutput {
                    invocation_epoch: claim.invocation_epoch,
                })
            }
            store::SideEffectLedgerPhase::Ambiguous {
                invocation_epoch, ..
            } => Ok(Self::IdleAmbiguous { invocation_epoch }),
            store::SideEffectLedgerPhase::IntentPersisted { .. }
            | store::SideEffectLedgerPhase::Failed { .. } => {
                Err(unsupported_side_effect_phase(phase))
            }
        }
    }
}

fn unsupported_side_effect_phase(phase: store::SideEffectLedgerPhase<'_>) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!(
        "unsupported side-effect driver phase: {}",
        side_effect_ledger_phase_name(phase)
    ))
}

fn side_effect_ledger_phase_name(phase: store::SideEffectLedgerPhase<'_>) -> &'static str {
    match phase {
        store::SideEffectLedgerPhase::IntentPersisted { .. } => "intent_persisted",
        store::SideEffectLedgerPhase::Claimed { .. } => "claimed",
        store::SideEffectLedgerPhase::Prepared { .. } => "prepared",
        store::SideEffectLedgerPhase::Started { .. } => "started",
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Observed { .. },
            ..
        } => "submission_observed",
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::NotSubmitted,
            ..
        } => "not_submitted",
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Unknown,
            ..
        } => "submission_unknown",
        store::SideEffectLedgerPhase::ReceiptObserved { .. } => "receipt_observed",
        store::SideEffectLedgerPhase::Confirmed { .. } => "confirmed",
        store::SideEffectLedgerPhase::Ambiguous { .. } => "ambiguous",
        store::SideEffectLedgerPhase::Failed { .. } => "failed",
    }
}

/// Runtime-minted claim authority used when building side-effect evidence for one invocation epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeSideEffectClaimAuthority {
    pub(crate) claim_owner: events::RunnerInvocationId,
    pub(crate) claim_generation: u32,
    pub(crate) claim_fencing_token: side_effect::ClaimFencingToken,
    pub(crate) resource_key: Option<events::ResourceKeyEvidence>,
}

impl RuntimeSideEffectClaimAuthority {
    fn claim_binding(&self) -> RunnerClaimBinding {
        RunnerClaimBinding {
            claim_owner: self.claim_owner.clone(),
            claim_generation: self.claim_generation,
            claim_fencing_token: self.claim_fencing_token.clone(),
        }
    }

    fn prepared_binding(&self) -> RunnerPreparedInvocationBinding {
        RunnerPreparedInvocationBinding {
            claim_generation: self.claim_generation,
            claim_fencing_token: self.claim_fencing_token.clone(),
            resource_key: self.resource_key.clone(),
        }
    }
}

/// Replay verifier evidence attached to receipt and confirmation observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectReplayEvidence {
    /// Replay verifier that can validate the observed evidence.
    pub replay_verifier_id: events::ReplayVerifierId,
    /// Optional exact touched resource set evidence.
    pub resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
}

/// Input for preparing and starting a side-effect invocation.
pub struct SideEffectPrepareEvidence<'a, Intent, Idempotency> {
    /// Side-effect ledger coordinates.
    pub(crate) side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub(crate) claim: RuntimeSideEffectClaimAuthority,
    /// Typed side-effect intent evidence.
    pub intent: &'a Intent,
    /// Typed idempotency input evidence.
    pub idempotency: &'a Idempotency,
    /// Stable idempotency key derived by the adapter.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability implementation that will perform the mutation.
    pub capability_binding: RunnerCapabilityBinding,
}

/// Input for preparing and starting a side-effect invocation with prepared invocation evidence.
pub struct SideEffectPreparedInvocationEvidence<'a, Intent, Idempotency, Prepared> {
    /// Side-effect ledger coordinates.
    pub(crate) side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub(crate) claim: RuntimeSideEffectClaimAuthority,
    /// Typed side-effect intent evidence.
    pub intent: &'a Intent,
    /// Typed idempotency input evidence.
    pub idempotency: &'a Idempotency,
    /// Stable idempotency key derived by the adapter.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability implementation that will perform the mutation.
    pub capability_binding: RunnerCapabilityBinding,
    /// Prepared invocation evidence. Live or secret-bearing data must not be included.
    pub prepared_invocation: &'a Prepared,
}

/// Input for the pre-invocation resource-lane claim commit.
pub struct SideEffectLaneClaimEvidence<'a, Intent, Idempotency> {
    /// Side-effect ledger coordinates.
    pub(crate) side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub(crate) claim: RuntimeSideEffectClaimAuthority,
    /// Typed side-effect intent evidence.
    pub intent: &'a Intent,
    /// Typed idempotency input evidence.
    pub idempotency: &'a Idempotency,
    /// Stable idempotency key derived by the adapter.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability implementation that will perform the mutation.
    pub capability_binding: RunnerCapabilityBinding,
    /// Resolved exclusive resource key.
    pub resource_key: events::ResourceKeyEvidence,
}

/// Adapter-owned preparation result recorded before crossing the uncertainty boundary.
pub struct SideEffectPreparedInvocationPlan<Prepared> {
    /// Public prepared invocation evidence. Live or secret-bearing data must not be included.
    pub prepared_invocation: Option<Prepared>,
}

impl<Prepared> SideEffectPreparedInvocationPlan<Prepared> {
    /// Creates a preparation result with no prepared artifact.
    pub fn none() -> Self {
        Self {
            prepared_invocation: None,
        }
    }

    /// Creates a preparation result with public prepared invocation evidence.
    pub fn with_prepared_invocation(prepared_invocation: Prepared) -> Self {
        Self {
            prepared_invocation: Some(prepared_invocation),
        }
    }
}

/// Owned side-effect intent and idempotency plan returned by adapter callbacks.
pub struct SideEffectIntentPlan<Intent, Idempotency> {
    /// Typed intent evidence.
    pub intent: Intent,
    /// Typed idempotency input evidence.
    pub idempotency: Idempotency,
    /// Stable idempotency key derived by the adapter.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability implementation that will perform the mutation.
    pub capability_binding: RunnerCapabilityBinding,
}

/// Submission recovery result returned by side-effect driver callbacks.
pub enum SideEffectSubmissionDecision<
    Submission,
    UnknownEvidence,
    NotSubmittedProof,
    AmbiguityEvidence,
> {
    /// Submission was observed and can be persisted.
    Observed(Submission),
    /// Submission status could not be determined and uncertainty evidence can be persisted.
    Unknown(UnknownEvidence),
    /// The invocation was proven not submitted.
    NotSubmitted(NotSubmittedProof),
    /// Submission recovery became ambiguous and must stop emitting further evidence.
    Ambiguous {
        /// Ambiguity classifier code.
        ambiguity_code: events::AmbiguityCode,
        /// Redaction-safe ambiguity evidence.
        evidence: AmbiguityEvidence,
    },
}

/// Boxed future returned by submission recovery callbacks.
pub type SideEffectSubmissionDecisionFuture<
    'a,
    Submission,
    UnknownEvidence,
    NotSubmittedProof,
    AmbiguityEvidence,
> = SideEffectDriverFuture<
    'a,
    SideEffectSubmissionDecision<Submission, UnknownEvidence, NotSubmittedProof, AmbiguityEvidence>,
>;

/// Observed side-effect evidence plus replay verifier metadata.
pub struct SideEffectObservedEvidence<T> {
    /// Typed observed evidence.
    pub evidence: T,
    /// Replay verifier metadata for this observation.
    pub replay: SideEffectReplayEvidence,
}

/// Adapter callbacks used by the generic side-effect driver.
///
/// Implementations own artifact reads, live providers, signing, and domain reconstruction. The
/// runtime driver uses callback results to build runner outputs through [`SideEffectEvidenceBuilder`]
/// without taking ownership of adapter IO.
pub trait SideEffectDriverCallbacks {
    /// Typed side-effect intent evidence.
    type Intent: MfmValue + Send + Sync + 'static;
    /// Typed idempotency input evidence.
    type Idempotency: MfmValue + Send + Sync + 'static;
    /// Public prepared invocation evidence.
    type PreparedInvocation: Serialize + Send + Sync + 'static;
    /// Typed submission evidence.
    type Submission: MfmValue + Send + Sync + 'static;
    /// Typed submission-unknown evidence.
    type SubmissionUnknownEvidence: MfmValue + Send + Sync + 'static;
    /// Typed not-submitted proof evidence.
    type NotSubmittedProof: MfmValue + Send + Sync + 'static;
    /// Typed receipt evidence.
    type Receipt: MfmValue + Send + Sync + 'static;
    /// Typed confirmation evidence.
    type Confirmation: MfmValue + Send + Sync + 'static;
    /// Typed ambiguity evidence.
    type AmbiguityEvidence: MfmValue + Send + Sync + 'static;
    /// Typed state output built from confirmation evidence.
    type Output: MfmValue + Send + Sync + 'static;

    /// Builds side-effect intent, idempotency, claim, and capability binding evidence.
    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>>;

    /// Resolves an exclusive resource lane using pure preflight data before live invocation work.
    fn resolve_resource_lane<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, Option<events::ResourceKeyEvidence>>;

    /// Prepares public invocation evidence before the external uncertainty boundary.
    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocationPlan<Self::PreparedInvocation>>;

    /// Reconstructs live invocation state from adapter-owned artifact reads.
    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation>;

    /// Submits the prepared invocation or recovers submission status after uncertainty.
    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        action: SideEffectProtocolAction,
        prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::SubmissionUnknownEvidence,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    >;

    /// Reads receipt evidence using adapter-owned artifact reads and live providers.
    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>>;

    /// Builds confirmation evidence using adapter-owned artifact reads.
    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>>;

    /// Maps stored confirmation evidence to the state output.
    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output>;
}

/// Generic one-step side-effect protocol driver.
///
/// The driver chooses one protocol action from a runtime-minted [`SideEffectAttemptView`] and
/// proposes one runner output. It never commits, derives store preconditions, reads artifacts, or
/// calls live providers directly.
pub struct SideEffectDriver;

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
            SideEffectProtocolAction::SubmitOrRecoverSubmission { invocation_epoch } => {
                Self::submit_or_recover_submission(&ctx, callbacks, &view, action, invocation_epoch)
                    .await
            }
            SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission {
                invocation_epoch,
            } => {
                Self::submit_or_recover_submission(&ctx, callbacks, &view, action, invocation_epoch)
                    .await
            }
            SideEffectProtocolAction::RetryAfterNotSubmitted {
                invocation_epoch,
                claim_generation,
            } => {
                Self::retry_after_not_submitted(
                    &ctx,
                    callbacks,
                    &view,
                    invocation_epoch,
                    claim_generation,
                )
                .await
            }
            SideEffectProtocolAction::ReadReceipt { invocation_epoch } => {
                let submission = observed_submission(&view)?;
                let receipt = callbacks.read_receipt(&ctx, submission).await?;
                SideEffectEvidenceBuilder::new(&ctx).receipt_observed(
                    side_effect_binding(&view, invocation_epoch)?,
                    &receipt.evidence,
                    receipt.replay,
                )
            }
            SideEffectProtocolAction::BuildConfirmation { invocation_epoch } => {
                let receipt = observed_receipt(&view)?;
                let confirmation = callbacks.build_confirmation(&ctx, receipt).await?;
                SideEffectEvidenceBuilder::new(&ctx).confirmation_observed(
                    side_effect_binding(&view, invocation_epoch)?,
                    &confirmation.evidence,
                    confirmation.replay,
                )
            }
            SideEffectProtocolAction::MapConfirmationToOutput { .. } => {
                let confirmation = observed_confirmation(&view)?;
                let output = callbacks
                    .map_confirmation_to_output(&ctx, confirmation)
                    .await?;
                build_side_effect_state_output(&ctx, &output)
            }
            SideEffectProtocolAction::IdleAmbiguous { .. } => {
                Ok(ErasedRunnerOutput::new(Vec::new()))
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
        let plan = callbacks.intent_and_idempotency(ctx).await?;
        let side_effect = runtime_side_effect_binding(ctx, &plan.idempotency_key)?;
        let resource_key = resolve_resource_lane(ctx, callbacks, &plan).await?;
        let claim = runtime_claim_authority(ctx, &side_effect, 1, resource_key.clone())?;
        if let Some(resource_key) = resource_key {
            return SideEffectEvidenceBuilder::new(ctx).claim_resource_lane(
                SideEffectLaneClaimEvidence {
                    side_effect,
                    claim,
                    intent: &plan.intent,
                    idempotency: &plan.idempotency,
                    idempotency_key: plan.idempotency_key,
                    capability_binding: plan.capability_binding,
                    resource_key,
                },
            );
        }
        let prepared = callbacks.prepare_invocation(ctx, &plan).await?;
        let builder = SideEffectEvidenceBuilder::new(ctx);
        if let Some(prepared) = prepared.prepared_invocation {
            builder.prepare_invocation_and_start(SideEffectPreparedInvocationEvidence {
                side_effect,
                claim,
                intent: &plan.intent,
                idempotency: &plan.idempotency,
                idempotency_key: plan.idempotency_key,
                capability_binding: plan.capability_binding,
                prepared_invocation: &prepared,
            })
        } else {
            builder.prepare_and_start(SideEffectPrepareEvidence {
                side_effect,
                claim,
                intent: &plan.intent,
                idempotency: &plan.idempotency,
                idempotency_key: plan.idempotency_key,
                capability_binding: plan.capability_binding,
            })
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
        let expected = runtime_side_effect_binding(ctx, &plan.idempotency_key)?;
        if expected.ledger_key != side_effect.ledger_key
            || expected.ledger_purpose != side_effect.ledger_purpose
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} resolved ledger identity that differs from the committed claim",
                ctx.node().node_id
            )));
        }
        let prepared = callbacks.prepare_invocation(ctx, &plan).await?;
        let claim = claimed_authority(view)?;
        let builder = SideEffectEvidenceBuilder::new(ctx);
        if let Some(prepared) = prepared.prepared_invocation {
            builder.prepare_claimed_invocation_and_start(side_effect, claim, Some(&prepared))
        } else {
            builder.prepare_claimed_invocation_and_start::<serde_json::Value>(
                side_effect,
                claim,
                None,
            )
        }
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
        let prepared = match prepared_invocation(view) {
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
                Some(prepared_claim_binding(view)?)
            }
            _ => None,
        };
        match decision {
            SideEffectSubmissionDecision::Observed(submission) => match start_claim {
                Some(claim) => {
                    builder.start_prepared_and_submission_observed(side_effect, claim, &submission)
                }
                None => builder.submission_observed(side_effect, &submission),
            },
            SideEffectSubmissionDecision::Unknown(evidence) => match start_claim {
                Some(claim) => {
                    builder.start_prepared_and_submission_unknown(side_effect, claim, &evidence)
                }
                None => builder.submission_unknown(side_effect, &evidence),
            },
            SideEffectSubmissionDecision::NotSubmitted(proof) => match start_claim {
                Some(claim) => {
                    builder.start_prepared_and_not_submitted_proven(side_effect, claim, &proof)
                }
                None => builder.not_submitted_proven(side_effect, &proof),
            },
            SideEffectSubmissionDecision::Ambiguous {
                ambiguity_code,
                evidence,
            } => match start_claim {
                Some(claim) => builder.start_prepared_and_ambiguous(
                    side_effect,
                    claim,
                    ambiguity_code,
                    &evidence,
                ),
                None => builder.ambiguous(side_effect, ambiguity_code, &evidence),
            },
        }
    }

    async fn retry_after_not_submitted<C>(
        ctx: &ErasedRunCtx<'_>,
        callbacks: &C,
        view: &SideEffectAttemptView<'_>,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectDriverCallbacks + ?Sized,
    {
        let plan = callbacks.intent_and_idempotency(ctx).await?;
        let side_effect = side_effect_binding(view, invocation_epoch)?;
        let resource_key = resolve_resource_lane(ctx, callbacks, &plan).await?;
        let claim =
            runtime_claim_authority(ctx, &side_effect, claim_generation, resource_key.clone())?;
        if let Some(resource_key) = resource_key {
            return SideEffectEvidenceBuilder::new(ctx).claim_resource_lane_without_intent(
                side_effect,
                claim,
                resource_key,
            );
        }
        let prepared = callbacks.prepare_invocation(ctx, &plan).await?;
        SideEffectEvidenceBuilder::new(ctx).claim_prepare_and_start(
            side_effect,
            claim,
            prepared.prepared_invocation.as_ref(),
        )
    }
}

fn side_effect_binding(
    view: &SideEffectAttemptView<'_>,
    invocation_epoch: u32,
) -> Result<RunnerSideEffectBinding> {
    Ok(RunnerSideEffectBinding {
        ledger_key: view
            .ledger_key()
            .cloned()
            .ok_or_else(|| missing_driver_projection("ledger key"))?,
        ledger_purpose: view
            .ledger_purpose()
            .cloned()
            .ok_or_else(|| missing_driver_projection("ledger purpose"))?,
        invocation_epoch,
    })
}

fn runtime_side_effect_binding(
    ctx: &ErasedRunCtx<'_>,
    idempotency_key: &events::IdempotencyKeyRef,
) -> Result<RunnerSideEffectBinding> {
    let ledger_purpose = runtime_ledger_purpose(ctx)?;
    let ledger_key = runtime_ledger_key(ctx, &ledger_purpose, idempotency_key)?;
    Ok(RunnerSideEffectBinding {
        ledger_key,
        ledger_purpose,
        invocation_epoch: 1,
    })
}

fn runtime_ledger_purpose(ctx: &ErasedRunCtx<'_>) -> Result<events::SideEffectLedgerPurpose> {
    Ok(linked_forward_ledger_for_remediation(ctx)?
        .map(
            |forward_ledger_key| events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key,
            },
        )
        .unwrap_or(events::SideEffectLedgerPurpose::Forward))
}

fn linked_forward_ledger_for_remediation(
    ctx: &ErasedRunCtx<'_>,
) -> Result<Option<events::SideEffectLedgerKey>> {
    if let Some(projection) = SideEffectAttemptView::from_erased_context(ctx)?.projection() {
        if let events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } =
            &projection.ledger_purpose
        {
            return Ok(Some(forward_ledger_key.clone()));
        }
    }
    let Some(forward_node_id) = ctx
        .runtime_spec()
        .forward_node_for_remediation(&ctx.node().node_id)
    else {
        return Ok(None);
    };
    Ok(ctx
        .projections()
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.intent.node_id == *forward_node_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
                && matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ))
            .then(|| projection.ledger_key.clone())
        }))
}

fn runtime_ledger_key(
    ctx: &ErasedRunCtx<'_>,
    ledger_purpose: &events::SideEffectLedgerPurpose,
    idempotency_key: &events::IdempotencyKeyRef,
) -> Result<events::SideEffectLedgerKey> {
    let digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "idempotency_key": idempotency_key.as_str(),
        "ledger_purpose": ledger_purpose_key(ledger_purpose),
        "node_id": ctx.node().node_id.as_str(),
        "run_id": ctx.run_id().as_str(),
        "spec_hash": ctx.spec_hash().as_str(),
    }))?;
    Ok(events::SideEffectLedgerKey::new(format!(
        "mfm.runtime.side_effect.{}",
        short_digest(&digest)
    ))?)
}

fn runtime_claim_authority(
    ctx: &ErasedRunCtx<'_>,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> Result<RuntimeSideEffectClaimAuthority> {
    let claim_owner = runtime_claim_owner(ctx, side_effect, claim_generation)?;
    let claim_fencing_token = runtime_claim_fencing_token(ctx, side_effect, claim_generation)?;
    Ok(RuntimeSideEffectClaimAuthority {
        claim_owner,
        claim_generation,
        claim_fencing_token,
        resource_key,
    })
}

fn runtime_claim_owner(
    ctx: &ErasedRunCtx<'_>,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
) -> Result<events::RunnerInvocationId> {
    let digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "run_id": ctx.run_id().as_str(),
    }))?;
    Ok(events::RunnerInvocationId::new(format!(
        "mfm.runtime.owner.{}",
        short_digest(&digest)
    ))?)
}

fn runtime_claim_fencing_token(
    ctx: &ErasedRunCtx<'_>,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
) -> Result<side_effect::ClaimFencingToken> {
    let digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "purpose": "side-effect-claim-fencing",
    }))?;
    Ok(side_effect::ClaimFencingToken::new(format!(
        "mfm.runtime.token.{}",
        short_digest(&digest)
    ))?)
}

async fn resolve_resource_lane<C, Intent, Idempotency>(
    ctx: &ErasedRunCtx<'_>,
    callbacks: &C,
    plan: &SideEffectIntentPlan<Intent, Idempotency>,
) -> Result<Option<events::ResourceKeyEvidence>>
where
    C: SideEffectDriverCallbacks<Intent = Intent, Idempotency = Idempotency> + ?Sized,
    Intent: MfmValue + Send + Sync + 'static,
    Idempotency: MfmValue + Send + Sync + 'static,
{
    let resource_key = callbacks.resolve_resource_lane(ctx, plan).await?;
    validate_resolved_resource_key(ctx, resource_key)
}

fn validate_resolved_resource_key(
    ctx: &ErasedRunCtx<'_>,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> Result<Option<events::ResourceKeyEvidence>> {
    let Some(side_effect_contract) = &ctx.node().side_effect else {
        if resource_key.is_some() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} recorded exclusive resource key without a side-effect contract",
                ctx.node().node_id
            )));
        }
        return Ok(resource_key);
    };
    match &side_effect_contract.resource_claim {
        spec::ResourceClaimSpec::Exclusive {
            namespace,
            key_schema,
        } => {
            let Some(resource_key) = resource_key else {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "exclusive side-effect node {} did not resolve concrete resource key evidence before invocation",
                    ctx.node().node_id
                )));
            };
            if &resource_key.namespace != namespace || &resource_key.key_schema_id != key_schema {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "exclusive side-effect node {} resolved resource key evidence outside certified schema",
                    ctx.node().node_id
                )));
            }
            Ok(Some(resource_key))
        }
        spec::ResourceClaimSpec::ExactTouchedSet { .. } | spec::ResourceClaimSpec::ManualOnly => {
            if resource_key.is_some() {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resolved exclusive resource key without an exclusive certified resource claim",
                    ctx.node().node_id
                )));
            }
            Ok(None)
        }
    }
}

fn resource_lane_requirement_digest(ctx: &ErasedRunCtx<'_>) -> Result<ContentDigest> {
    let Some(spec::SideEffectContractSpec {
        resource_claim:
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        ..
    }) = &ctx.node().side_effect
    else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} tried to claim a resource lane without an exclusive resource contract",
            ctx.node().node_id
        )));
    };
    crate::content_digest_json(serde_json::json!({
        "acquisition": "pre_state_invocation",
        "hold": "until_side_effect_terminal",
        "key_schema_id": key_schema.as_str(),
        "mode": "exclusive",
        "namespace": namespace.as_str(),
    }))
}

fn ledger_purpose_key(ledger_purpose: &events::SideEffectLedgerPurpose) -> serde_json::Value {
    match ledger_purpose {
        events::SideEffectLedgerPurpose::Forward => serde_json::json!({
            "kind": "forward",
        }),
        events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => serde_json::json!({
            "forward_ledger_key": forward_ledger_key.as_str(),
            "kind": "remediation",
        }),
    }
}

fn short_digest(digest: &ContentDigest) -> String {
    digest
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or(digest.as_str())
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(32)
        .collect()
}

fn prepared_invocation<'view>(
    view: &SideEffectAttemptView<'view>,
) -> Option<&'view store::SideEffectArtifactProjection> {
    view.projection()
        .and_then(|projection| projection.prepared_invocation.as_ref())
}

fn prepared_claim_binding(view: &SideEffectAttemptView<'_>) -> Result<RunnerClaimBinding> {
    match view.phase() {
        Some(store::SideEffectLedgerPhase::Prepared { claim, .. }) => Ok(RunnerClaimBinding {
            claim_owner: claim.claim_owner.clone(),
            claim_generation: claim.claim_generation,
            claim_fencing_token: claim.claim_fencing_token.clone(),
        }),
        _ => Err(missing_driver_projection("prepared claim")),
    }
}

fn claimed_authority(view: &SideEffectAttemptView<'_>) -> Result<RuntimeSideEffectClaimAuthority> {
    let projection = view
        .projection()
        .ok_or_else(|| missing_driver_projection("claimed side-effect projection"))?;
    match view.phase() {
        Some(store::SideEffectLedgerPhase::Claimed { claim }) => {
            Ok(RuntimeSideEffectClaimAuthority {
                claim_owner: claim.claim_owner.clone(),
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token.clone(),
                resource_key: projection.resource_key.clone(),
            })
        }
        _ => Err(missing_driver_projection("claimed authority")),
    }
}

fn observed_submission<'view>(
    view: &SideEffectAttemptView<'view>,
) -> Result<&'view store::SideEffectArtifactProjection> {
    match view.phase() {
        Some(store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Observed { submission },
            ..
        }) => Ok(submission),
        _ => Err(missing_driver_projection("submission")),
    }
}

fn observed_receipt<'view>(
    view: &SideEffectAttemptView<'view>,
) -> Result<&'view store::SideEffectArtifactProjection> {
    match view.phase() {
        Some(store::SideEffectLedgerPhase::ReceiptObserved { receipt, .. }) => Ok(receipt),
        _ => Err(missing_driver_projection("receipt")),
    }
}

fn observed_confirmation<'view>(
    view: &SideEffectAttemptView<'view>,
) -> Result<&'view store::SideEffectArtifactProjection> {
    match view.phase() {
        Some(store::SideEffectLedgerPhase::Confirmed { confirmation, .. }) => Ok(confirmation),
        _ => Err(missing_driver_projection("confirmation")),
    }
}

fn missing_driver_projection(label: &str) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!(
        "side-effect driver missing verified {label} projection"
    ))
}

fn build_side_effect_state_output<Output>(
    ctx: &ErasedRunCtx<'_>,
    output: &Output,
) -> Result<ErasedRunnerOutput>
where
    Output: MfmValue,
{
    let artifacts = RunnerArtifactBuilder::new(ctx);
    let payloads = RunnerPayloadBuilder::new(ctx);
    let artifact = artifacts.state_output(output)?;
    let mut runner_output = RunnerOutputBuilder::new(ctx);
    runner_output.stage_attempt_artifact(&artifact)?;
    runner_output.retain_runtime_evidence(&artifact);
    runner_output.payload(payloads.cell_produced(&artifact)?);
    Ok(runner_output.finish())
}

/// Builder for side-effect runner evidence outputs.
///
/// This builder stages artifacts and emits runner payloads only. It does not derive commit
/// preconditions, append commits, perform live IO, read prior artifacts, or create attempt
/// failures.
pub struct SideEffectEvidenceBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
}

impl<'a, 'ctx> SideEffectEvidenceBuilder<'a, 'ctx> {
    /// Creates a side-effect evidence builder for one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Builds intent, side-effect claim, and resource-lane claim evidence.
    pub fn claim_resource_lane<Intent, Idempotency>(
        &self,
        evidence: SideEffectLaneClaimEvidence<'_, Intent, Idempotency>,
    ) -> Result<ErasedRunnerOutput>
    where
        Intent: MfmValue,
        Idempotency: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let intent_artifact = artifacts.side_effect_intent(evidence.intent)?;
        let side_effect = evidence.side_effect;
        let claim = evidence.claim;

        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.stage_side_effect_artifact(
            &intent_artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        output.retain_runtime_evidence(&intent_artifact);
        output.payload(payloads.side_effect_intent_persisted(
            side_effect.clone(),
            &intent_artifact,
            evidence.idempotency,
            evidence.idempotency_key,
            evidence.capability_binding,
        )?);
        output.payload(payloads.side_effect_claimed(side_effect.clone(), claim.claim_binding()));
        output
            .payload(self.resource_lane_claim_intent_payload(side_effect, evidence.resource_key)?);
        Ok(output.finish())
    }

    /// Builds side-effect claim and resource-lane claim evidence for a retry epoch.
    pub fn claim_resource_lane_without_intent(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RuntimeSideEffectClaimAuthority,
        resource_key: events::ResourceKeyEvidence,
    ) -> Result<ErasedRunnerOutput> {
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.payload(payloads.side_effect_claimed(side_effect.clone(), claim.claim_binding()));
        output.payload(self.resource_lane_claim_intent_payload(side_effect, resource_key)?);
        Ok(output.finish())
    }

    /// Builds prepared invocation and started evidence for an already claimed lane.
    pub fn prepare_claimed_invocation_and_start<Prepared>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RuntimeSideEffectClaimAuthority,
        prepared_invocation: Option<&Prepared>,
    ) -> Result<ErasedRunnerOutput>
    where
        Prepared: Serialize,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let prepared_artifact = prepared_invocation
            .map(|prepared| artifacts.prepared_invocation(prepared))
            .transpose()?;
        let mut output = RunnerOutputBuilder::new(self.ctx);
        if let Some(prepared_artifact) = &prepared_artifact {
            output.stage_side_effect_artifact(
                prepared_artifact,
                side_effect.ledger_key.clone(),
                side_effect.invocation_epoch,
            )?;
            if !self
                .ctx
                .projections()
                .retention(self.ctx.run_id())
                .is_some_and(|retention| {
                    retention
                        .refs
                        .contains_key(&prepared_artifact.evidence().artifact_id)
                })
            {
                output.retain_runtime_evidence(prepared_artifact);
            }
        }
        output.payload(payloads.side_effect_invocation_prepared(
            side_effect.clone(),
            prepared_artifact.as_ref(),
            claim.prepared_binding(),
        )?);
        output.payload(payloads.side_effect_invocation_started(side_effect, claim.claim_binding()));
        Ok(output.finish())
    }

    /// Builds intent, claim, prepared-without-artifact, and started evidence.
    pub fn prepare_and_start<Intent, Idempotency>(
        &self,
        evidence: SideEffectPrepareEvidence<'_, Intent, Idempotency>,
    ) -> Result<ErasedRunnerOutput>
    where
        Intent: MfmValue,
        Idempotency: MfmValue,
    {
        self.prepare_and_start_with_artifact(evidence, None)
    }

    /// Builds intent, claim, prepared-with-artifact, and started evidence.
    pub fn prepare_invocation_and_start<Intent, Idempotency, Prepared>(
        &self,
        evidence: SideEffectPreparedInvocationEvidence<'_, Intent, Idempotency, Prepared>,
    ) -> Result<ErasedRunnerOutput>
    where
        Intent: MfmValue,
        Idempotency: MfmValue,
        Prepared: Serialize,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let prepared_artifact = artifacts.prepared_invocation(evidence.prepared_invocation)?;
        let evidence = SideEffectPrepareEvidence {
            side_effect: evidence.side_effect,
            claim: evidence.claim,
            intent: evidence.intent,
            idempotency: evidence.idempotency,
            idempotency_key: evidence.idempotency_key,
            capability_binding: evidence.capability_binding,
        };
        self.prepare_and_start_with_artifact(evidence, Some(prepared_artifact))
    }

    /// Builds not-submitted proof evidence.
    pub fn not_submitted_proven<Proof>(
        &self,
        side_effect: RunnerSideEffectBinding,
        proof: &Proof,
    ) -> Result<ErasedRunnerOutput>
    where
        Proof: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.not_submitted_proof(proof)?;
        self.terminal_single_artifact_output(
            side_effect,
            None,
            &artifact,
            "side_effect.not_submitted",
            |payloads, side_effect, artifact| {
                payloads.side_effect_not_submitted_proven(side_effect, artifact)
            },
        )
    }

    /// Builds invocation-started and not-submitted proof evidence for a prepared invocation.
    pub fn start_prepared_and_not_submitted_proven<Proof>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
        proof: &Proof,
    ) -> Result<ErasedRunnerOutput>
    where
        Proof: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.not_submitted_proof(proof)?;
        self.terminal_single_artifact_output(
            side_effect,
            Some(claim),
            &artifact,
            "side_effect.not_submitted",
            |payloads, side_effect, artifact| {
                payloads.side_effect_not_submitted_proven(side_effect, artifact)
            },
        )
    }

    /// Builds submission observed evidence.
    pub fn submission_observed<Submission>(
        &self,
        side_effect: RunnerSideEffectBinding,
        submission: &Submission,
    ) -> Result<ErasedRunnerOutput>
    where
        Submission: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission(submission)?;
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_submission_observed(side_effect, artifact)
        })
    }

    /// Builds invocation-started and submission-observed evidence for a prepared invocation.
    pub fn start_prepared_and_submission_observed<Submission>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
        submission: &Submission,
    ) -> Result<ErasedRunnerOutput>
    where
        Submission: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission(submission)?;
        self.started_single_artifact_output(
            side_effect,
            claim,
            &artifact,
            |payloads, side_effect, artifact| {
                payloads.side_effect_submission_observed(side_effect, artifact)
            },
        )
    }

    /// Builds submission unknown evidence.
    pub fn submission_unknown<Evidence>(
        &self,
        side_effect: RunnerSideEffectBinding,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission_unknown(evidence)?;
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_submission_unknown(side_effect, artifact)
        })
    }

    /// Builds invocation-started and submission-unknown evidence for a prepared invocation.
    pub fn start_prepared_and_submission_unknown<Evidence>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission_unknown(evidence)?;
        self.started_single_artifact_output(
            side_effect,
            claim,
            &artifact,
            |payloads, side_effect, artifact| {
                payloads.side_effect_submission_unknown(side_effect, artifact)
            },
        )
    }

    /// Builds receipt observed evidence.
    pub fn receipt_observed<Receipt>(
        &self,
        side_effect: RunnerSideEffectBinding,
        receipt: &Receipt,
        replay: SideEffectReplayEvidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Receipt: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.receipt(receipt)?;
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_receipt_observed(
                side_effect,
                artifact,
                replay.replay_verifier_id,
                replay.resource_touched_set,
            )
        })
    }

    /// Builds confirmation observed evidence.
    pub fn confirmation_observed<Confirmation>(
        &self,
        side_effect: RunnerSideEffectBinding,
        confirmation: &Confirmation,
        replay: SideEffectReplayEvidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Confirmation: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.confirmation(confirmation)?;
        self.terminal_single_artifact_output(
            side_effect,
            None,
            &artifact,
            "side_effect.confirmed",
            |payloads, side_effect, artifact| {
                payloads.side_effect_confirmation_observed(
                    side_effect,
                    artifact,
                    replay.replay_verifier_id,
                    replay.resource_touched_set,
                )
            },
        )
    }

    /// Builds ambiguity evidence.
    pub fn ambiguous<Evidence>(
        &self,
        side_effect: RunnerSideEffectBinding,
        ambiguity_code: events::AmbiguityCode,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.ambiguity_evidence(evidence)?;
        self.terminal_single_artifact_output(
            side_effect,
            None,
            &artifact,
            "side_effect.ambiguous",
            |payloads, side_effect, artifact| {
                payloads.side_effect_ambiguous(side_effect, ambiguity_code, artifact)
            },
        )
    }

    /// Builds invocation-started and ambiguity evidence for a prepared invocation.
    pub fn start_prepared_and_ambiguous<Evidence>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
        ambiguity_code: events::AmbiguityCode,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.ambiguity_evidence(evidence)?;
        self.terminal_single_artifact_output(
            side_effect,
            Some(claim),
            &artifact,
            "side_effect.ambiguous",
            |payloads, side_effect, artifact| {
                payloads.side_effect_ambiguous(side_effect, ambiguity_code, artifact)
            },
        )
    }

    /// Builds a new claim, optional prepared invocation evidence, and invocation start evidence.
    pub fn claim_prepare_and_start<Prepared>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RuntimeSideEffectClaimAuthority,
        prepared_invocation: Option<&Prepared>,
    ) -> Result<ErasedRunnerOutput>
    where
        Prepared: Serialize,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let prepared_artifact = prepared_invocation
            .map(|prepared| artifacts.prepared_invocation(prepared))
            .transpose()?;

        let mut output = RunnerOutputBuilder::new(self.ctx);
        if let Some(prepared_artifact) = &prepared_artifact {
            output.stage_side_effect_artifact(
                prepared_artifact,
                side_effect.ledger_key.clone(),
                side_effect.invocation_epoch,
            )?;
            if !self
                .ctx
                .projections()
                .retention(self.ctx.run_id())
                .is_some_and(|retention| {
                    retention
                        .refs
                        .contains_key(&prepared_artifact.evidence().artifact_id)
                })
            {
                output.retain_runtime_evidence(prepared_artifact);
            }
        }
        output.payload(payloads.side_effect_claimed(side_effect.clone(), claim.claim_binding()));
        output.payload(payloads.side_effect_invocation_prepared(
            side_effect.clone(),
            prepared_artifact.as_ref(),
            claim.prepared_binding(),
        )?);
        output.payload(payloads.side_effect_invocation_started(side_effect, claim.claim_binding()));
        Ok(output.finish())
    }

    fn prepare_and_start_with_artifact<Intent, Idempotency>(
        &self,
        evidence: SideEffectPrepareEvidence<'_, Intent, Idempotency>,
        prepared_artifact: Option<RunnerJsonArtifact>,
    ) -> Result<ErasedRunnerOutput>
    where
        Intent: MfmValue,
        Idempotency: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let intent_artifact = artifacts.side_effect_intent(evidence.intent)?;
        let side_effect = evidence.side_effect;
        let claim = evidence.claim;

        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.stage_side_effect_artifact(
            &intent_artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        output.retain_runtime_evidence(&intent_artifact);
        if let Some(prepared_artifact) = &prepared_artifact {
            output.stage_side_effect_artifact(
                prepared_artifact,
                side_effect.ledger_key.clone(),
                side_effect.invocation_epoch,
            )?;
            output.retain_runtime_evidence(prepared_artifact);
        }
        output.payload(payloads.side_effect_intent_persisted(
            side_effect.clone(),
            &intent_artifact,
            evidence.idempotency,
            evidence.idempotency_key,
            evidence.capability_binding,
        )?);
        output.payload(payloads.side_effect_claimed(side_effect.clone(), claim.claim_binding()));
        output.payload(payloads.side_effect_invocation_prepared(
            side_effect.clone(),
            prepared_artifact.as_ref(),
            claim.prepared_binding(),
        )?);
        output.payload(payloads.side_effect_invocation_started(side_effect, claim.claim_binding()));
        Ok(output.finish())
    }

    fn resource_lane_claim_intent_payload(
        &self,
        side_effect: RunnerSideEffectBinding,
        resource_key: events::ResourceKeyEvidence,
    ) -> Result<crate::RunnerEventPayload> {
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let requirement_digest = resource_lane_requirement_digest(self.ctx)?;
        let resolved_by = events::RunnerFactoryId::new(self.ctx.descriptor().runner.as_str())?;
        Ok(payloads.resource_lane_claim_intent(
            side_effect,
            resource_key,
            requirement_digest,
            resolved_by,
        ))
    }

    fn single_artifact_output<F>(
        &self,
        side_effect: RunnerSideEffectBinding,
        artifact: &RunnerJsonArtifact,
        payload: F,
    ) -> Result<ErasedRunnerOutput>
    where
        F: FnOnce(
            &RunnerPayloadBuilder<'_, '_>,
            RunnerSideEffectBinding,
            &RunnerJsonArtifact,
        ) -> Result<crate::RunnerEventPayload>,
    {
        self.single_artifact_output_with_optional_start(side_effect, None, artifact, None, payload)
    }

    fn started_single_artifact_output<F>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
        artifact: &RunnerJsonArtifact,
        payload: F,
    ) -> Result<ErasedRunnerOutput>
    where
        F: FnOnce(
            &RunnerPayloadBuilder<'_, '_>,
            RunnerSideEffectBinding,
            &RunnerJsonArtifact,
        ) -> Result<crate::RunnerEventPayload>,
    {
        self.single_artifact_output_with_optional_start(
            side_effect,
            Some(claim),
            artifact,
            None,
            payload,
        )
    }

    fn terminal_single_artifact_output<F>(
        &self,
        side_effect: RunnerSideEffectBinding,
        start_claim: Option<RunnerClaimBinding>,
        artifact: &RunnerJsonArtifact,
        release_reason: &'static str,
        payload: F,
    ) -> Result<ErasedRunnerOutput>
    where
        F: FnOnce(
            &RunnerPayloadBuilder<'_, '_>,
            RunnerSideEffectBinding,
            &RunnerJsonArtifact,
        ) -> Result<crate::RunnerEventPayload>,
    {
        self.single_artifact_output_with_optional_start(
            side_effect,
            start_claim,
            artifact,
            Some(release_reason),
            payload,
        )
    }

    fn single_artifact_output_with_optional_start<F>(
        &self,
        side_effect: RunnerSideEffectBinding,
        start_claim: Option<RunnerClaimBinding>,
        artifact: &RunnerJsonArtifact,
        release_reason: Option<&'static str>,
        payload: F,
    ) -> Result<ErasedRunnerOutput>
    where
        F: FnOnce(
            &RunnerPayloadBuilder<'_, '_>,
            RunnerSideEffectBinding,
            &RunnerJsonArtifact,
        ) -> Result<crate::RunnerEventPayload>,
    {
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.stage_side_effect_artifact(
            artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        output.retain_runtime_evidence(artifact);
        if let Some(claim) = start_claim {
            output.payload(payloads.side_effect_invocation_started(side_effect.clone(), claim));
        }
        if let Some(release_reason) = release_reason {
            if let Some(release) =
                self.resource_lane_released_payload(side_effect.clone(), release_reason)?
            {
                output.payload(release);
            }
        }
        output.payload(payload(&payloads, side_effect, artifact)?);
        Ok(output.finish())
    }

    fn resource_lane_released_payload(
        &self,
        side_effect: RunnerSideEffectBinding,
        release_reason: &'static str,
    ) -> Result<Option<crate::RunnerEventPayload>> {
        let holder = store::SideEffectLedgerRef::new(
            self.ctx.run_id().clone(),
            side_effect.ledger_key.clone(),
        );
        let Some((_, lane)) = self
            .ctx
            .projections()
            .resource_lanes()
            .find(|(_, projection)| projection.holder == holder)
        else {
            return Ok(None);
        };
        if lane.node_id != self.ctx.node().node_id
            || lane.attempt_id != *self.ctx.attempt_id()
            || lane.ledger_purpose != side_effect.ledger_purpose
            || lane.invocation_epoch != side_effect.invocation_epoch
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "active resource lane for ledger {} does not match side-effect terminal context",
                side_effect.ledger_key
            )));
        }
        let release_reason = events::ResourceLaneReleaseReason::new(release_reason)?;
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        Ok(Some(payloads.resource_lane_release_intent(
            side_effect,
            lane.claim_id.clone(),
            release_reason,
        )))
    }
}
