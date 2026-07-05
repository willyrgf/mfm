use std::future::Future;
use std::pin::Pin;

use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    short_stable_id_fragment, ArtifactId, AttemptId, ContentDigest, NodeId, RunId, SideEffectPairId,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::Serialize;

use crate::runner_kit::{
    RunnerClaimBinding, RunnerPreparedInvocationBinding, RunnerSideEffectBinding,
};
use crate::{
    canonical_json, CertifiedRuntimeSpec, ErasedRunCtx, ErasedRunnerOutput, MaterializedInputs,
    PreInvocationRunCtx, Result, RunnerArtifactBuilder, RunnerCapabilityBinding,
    RunnerEventPayload, RunnerJsonArtifact, RunnerOutputBuilder, RunnerPayloadBuilder,
    RuntimeError, SideEffectAttemptView, StagedArtifact, StagedRetentionRefs,
};

/// Boxed future returned by side-effect driver callbacks.
pub type SideEffectDriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Builder for pre-invocation side-effect resource-lane claim evidence.
pub struct SideEffectLanePreclaimBuilder<'a> {
    ctx: &'a PreInvocationRunCtx<'a>,
}

impl<'a> SideEffectLanePreclaimBuilder<'a> {
    /// Creates a builder for one pre-invocation context.
    pub fn new(ctx: &'a PreInvocationRunCtx<'a>) -> Self {
        Self { ctx }
    }

    /// Builds pre-invocation intent/claim evidence for the first epoch.
    pub fn claim_resource_lane<Intent, Idempotency>(
        &self,
        intent: &Intent,
        idempotency: &Idempotency,
        idempotency_key: events::IdempotencyKeyRef,
        capability_binding: RunnerCapabilityBinding,
        resource_key: events::ResourceKeyEvidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Intent: MfmValue,
        Idempotency: MfmValue,
    {
        let (side_effect, claim) = pre_invocation_lane_claim(self.ctx, Some(resource_key.clone()))?;
        let intent_artifact = pre_invocation_value_artifact(
            self.ctx,
            intent,
            events::ArtifactRole::SideEffectIntent,
        )?;
        let staged_artifact = StagedArtifact::inline_pre_invocation_side_effect_artifact(
            self.ctx,
            intent_artifact.bytes.clone(),
            intent_artifact.evidence.clone(),
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        Ok(ErasedRunnerOutput {
            staged_artifacts: vec![staged_artifact],
            staged_retention_refs: vec![StagedRetentionRefs::runtime_evidence(vec![
                intent_artifact.evidence.retention_ref(),
            ])],
            payloads: vec![
                pre_invocation_side_effect_intent_persisted(
                    self.ctx,
                    side_effect.clone(),
                    &intent_artifact,
                    idempotency,
                    idempotency_key,
                    capability_binding,
                )?,
                pre_invocation_side_effect_claimed(
                    self.ctx,
                    side_effect.clone(),
                    claim.claim_binding(),
                ),
                pre_invocation_resource_lane_claim_intent(self.ctx, side_effect, resource_key)?,
            ],
        })
    }
}

struct PreInvocationJsonArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn pre_invocation_value_artifact<T>(
    ctx: &PreInvocationRunCtx<'_>,
    value: &T,
    role: events::ArtifactRole,
) -> Result<PreInvocationJsonArtifact>
where
    T: MfmValue,
{
    let bytes = canonical_mfm_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(
            T::schema_id().map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?,
        ),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(PreInvocationJsonArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn pre_invocation_side_effect_intent_persisted<Idempotency>(
    ctx: &PreInvocationRunCtx<'_>,
    side_effect: RunnerSideEffectBinding,
    intent: &PreInvocationJsonArtifact,
    idempotency: &Idempotency,
    idempotency_key: events::IdempotencyKeyRef,
    binding: RunnerCapabilityBinding,
) -> Result<RunnerEventPayload>
where
    Idempotency: MfmValue,
{
    if intent.evidence.artifact_role != events::ArtifactRole::SideEffectIntent {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "artifact {} is not side-effect intent evidence",
            intent.evidence.artifact_id
        )));
    }
    Ok(RunnerEventPayload::SideEffectIntentPersisted(
        side_effect::IntentPersisted {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            scope_id: ctx.node().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: side_effect.invocation_epoch,
            intent_schema_id: intent.evidence.schema_id.clone().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "artifact {} missing required schema id",
                    intent.evidence.artifact_id
                ))
            })?,
            intent_hash: intent.evidence.digest.clone(),
            intent_artifact_id: intent.evidence.artifact_id.clone(),
            idempotency_input_schema_id: Idempotency::schema_id()
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?,
            idempotency_input_hash: canonical_mfm_value(idempotency)?.content_digest(),
            idempotency_key,
            capability_kind: binding.capability_kind,
            capability_version: binding.capability_version,
            adapter_kind: binding.adapter_kind,
            adapter_version: binding.adapter_version,
        },
    ))
}

fn pre_invocation_side_effect_claimed(
    ctx: &PreInvocationRunCtx<'_>,
    side_effect: RunnerSideEffectBinding,
    claim: RunnerClaimBinding,
) -> RunnerEventPayload {
    RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        ledger_key: side_effect.ledger_key.clone(),
        ledger_purpose: side_effect.ledger_purpose.clone(),
        pair_id: side_effect.pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        claim_owner: claim.claim_owner,
        invocation_epoch: side_effect.invocation_epoch,
        claim_generation: claim.claim_generation,
        claim_fencing_token: claim.claim_fencing_token,
    })
}

fn pre_invocation_resource_lane_claim_intent(
    ctx: &PreInvocationRunCtx<'_>,
    side_effect: RunnerSideEffectBinding,
    resource_key: events::ResourceKeyEvidence,
) -> Result<RunnerEventPayload> {
    let resource_key = validate_pre_invocation_resolved_resource_key(ctx, Some(resource_key))?
        .expect("resource key present");
    Ok(RunnerEventPayload::ResourceLaneClaimIntent(
        events::ResourceLaneClaimIntent {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: side_effect.invocation_epoch,
            resource_key,
            requirement_digest: pre_invocation_resource_lane_requirement_digest(ctx)?,
            resolved_by_capability_impl: events::RunnerFactoryId::new(
                ctx.descriptor().runner.as_str(),
            )?,
        },
    ))
}

fn validate_pre_invocation_resolved_resource_key(
    ctx: &PreInvocationRunCtx<'_>,
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
    /// A durable submission boundary result exists; complete the submit anchor.
    CompleteSubmissionBoundary {
        /// Invocation epoch whose boundary result was recorded.
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
            store::SideEffectLedgerPhase::Started { claim, .. } => {
                Ok(Self::SubmitOrRecoverSubmission {
                    invocation_epoch: claim.invocation_epoch,
                })
            }
            store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::NotSubmitted,
            } => Ok(Self::CompleteSubmissionBoundary {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::Observed { .. },
            }
            | store::SideEffectLedgerPhase::SubmissionKnown {
                claim,
                status: store::SideEffectSubmissionState::Unknown,
            } => Ok(Self::CompleteSubmissionBoundary {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::IntentPersisted { .. }
            | store::SideEffectLedgerPhase::ReceiptObserved { .. }
            | store::SideEffectLedgerPhase::Confirmed { .. }
            | store::SideEffectLedgerPhase::Ambiguous { .. }
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

/// Verify-side recovery result for a previously unknown submission.
pub enum SideEffectUnknownSubmissionDecision<Submission, NotSubmittedProof, AmbiguityEvidence> {
    /// Submission was observed and can be persisted by the verify role.
    Observed(Submission),
    /// Submission status remains unknown; the scheduler must stop without committing evidence.
    StillUnknown,
    /// The invocation was proven not submitted by the verify role.
    NotSubmitted(NotSubmittedProof),
    /// Recovery became ambiguous and must stop emitting further evidence.
    Ambiguous {
        /// Ambiguity classifier code.
        ambiguity_code: events::AmbiguityCode,
        /// Redaction-safe ambiguity evidence.
        evidence: AmbiguityEvidence,
    },
}

/// Boxed future returned by verify-side unknown-submission recovery callbacks.
pub type SideEffectUnknownSubmissionDecisionFuture<
    'a,
    Submission,
    NotSubmittedProof,
    AmbiguityEvidence,
> = SideEffectDriverFuture<
    'a,
    SideEffectUnknownSubmissionDecision<Submission, NotSubmittedProof, AmbiguityEvidence>,
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
    /// Typed ambiguity evidence.
    type AmbiguityEvidence: MfmValue + Send + Sync + 'static;

    /// Builds side-effect intent, idempotency, claim, and capability binding evidence.
    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>>;

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
                Some(prepared_claim_binding(view)?)
            }
            _ => None,
        };
        match decision {
            SideEffectSubmissionDecision::Observed(submission) => {
                builder.submission_observed(side_effect, start_claim, &submission)
            }
            SideEffectSubmissionDecision::Unknown(evidence) => {
                builder.submission_unknown(side_effect, start_claim, &evidence)
            }
            SideEffectSubmissionDecision::NotSubmitted(proof) => {
                builder.not_submitted_proven(side_effect, start_claim, &proof)
            }
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

/// Adapter callbacks used by the generic side-effect verify driver.
pub trait SideEffectVerifyCallbacks {
    /// Typed submission evidence recovered by the verify role.
    type Submission: MfmValue + Send + Sync + 'static;
    /// Typed receipt evidence.
    type Receipt: MfmValue + Send + Sync + 'static;
    /// Typed confirmation evidence.
    type Confirmation: MfmValue + Send + Sync + 'static;
    /// Typed state output built from terminal verification evidence.
    type Output: MfmValue + Send + Sync + 'static;
    /// Typed not-submitted proof evidence recovered by the verify role.
    type NotSubmittedProof: MfmValue + Send + Sync + 'static;
    /// Typed ambiguity evidence recovered by the verify role.
    type AmbiguityEvidence: MfmValue + Send + Sync + 'static;

    /// Recovers a previously unknown submission without crossing the submission boundary again.
    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectUnknownSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    >;

    /// Reads receipt evidence for an observed submission.
    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>>;

    /// Builds confirmation evidence from a stored receipt.
    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>>;

    /// Maps stored receipt evidence to the verified state output.
    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output>;

    /// Maps stored confirmation evidence to the verified state output.
    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output>;
}

/// Generic one-step side-effect verification driver.
pub struct SideEffectVerifyDriver;

impl SideEffectVerifyDriver {
    /// Drives one verification step for a certified side-effect verify framework node.
    pub async fn drive<C>(ctx: ErasedRunCtx<'_>, callbacks: &C) -> Result<ErasedRunnerOutput>
    where
        C: SideEffectVerifyCallbacks + ?Sized,
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
                    .read_receipt(&ctx, submit_node, &submit_inputs, submission)
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
            } => build_side_effect_failed_with_release(
                &ctx,
                side_effect,
                side_effect::FailurePhase::AfterNotSubmittedProven,
                false,
                "side-effect invocation was proven not submitted",
            ),
            store::SideEffectLedgerPhase::SubmissionKnown {
                status: store::SideEffectSubmissionState::Unknown,
                ..
            } => {
                let decision = callbacks
                    .recover_unknown_submission(
                        &ctx,
                        submit_node,
                        &submit_inputs,
                        projection.prepared_invocation.as_ref(),
                    )
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
            store::SideEffectLedgerPhase::ReceiptObserved { receipt, .. } => {
                match &submit_contract.verification {
                    spec::SideEffectVerificationSpec::Receipt => {
                        let output = callbacks
                            .map_receipt_to_output(&ctx, submit_node, &submit_inputs, receipt)
                            .await?;
                        ErasedRunnerOutput::state_output(&ctx, &output)
                    }
                    spec::SideEffectVerificationSpec::Finalized { .. } => {
                        let confirmation = callbacks
                            .build_confirmation(&ctx, submit_node, &submit_inputs, receipt)
                            .await?;
                        SideEffectEvidenceBuilder::new(&ctx).confirmation_observed(
                            side_effect,
                            &confirmation.evidence,
                            confirmation.replay,
                        )
                    }
                }
            }
            store::SideEffectLedgerPhase::Confirmed {
                receipt,
                confirmation,
                ..
            } => match &submit_contract.verification {
                spec::SideEffectVerificationSpec::Receipt => {
                    let output = callbacks
                        .map_receipt_to_output(&ctx, submit_node, &submit_inputs, receipt)
                        .await?;
                    ErasedRunnerOutput::state_output(&ctx, &output)
                }
                spec::SideEffectVerificationSpec::Finalized { .. } => {
                    let output = callbacks
                        .map_confirmation_to_output(&ctx, submit_node, &submit_inputs, confirmation)
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

fn side_effect_binding(
    view: &SideEffectAttemptView<'_>,
    invocation_epoch: u32,
) -> Result<RunnerSideEffectBinding> {
    let projection = view
        .projection()
        .ok_or_else(|| missing_driver_projection("side-effect projection"))?;
    Ok(RunnerSideEffectBinding {
        ledger_key: projection.ledger_key.clone(),
        ledger_purpose: projection.ledger_purpose.clone(),
        pair_id: projection.pair_id.clone(),
        invocation_epoch,
    })
}

fn runtime_side_effect_binding(ctx: &ErasedRunCtx<'_>) -> Result<RunnerSideEffectBinding> {
    let linked_forward_pair = match SideEffectAttemptView::from_erased_context(ctx)?.projection() {
        Some(projection) => match &projection.ledger_purpose {
            events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                Some(forward_pair_id.clone())
            }
            _ => terminal_forward_pair_for_remediation(
                ctx.runtime_spec(),
                ctx.projections(),
                ctx.node(),
            )?,
        },
        None => terminal_forward_pair_for_remediation(
            ctx.runtime_spec(),
            ctx.projections(),
            ctx.node(),
        )?,
    };
    let ledger_purpose = linked_forward_pair
        .map(|forward_pair_id| events::SideEffectLedgerPurpose::Remediation { forward_pair_id })
        .unwrap_or(events::SideEffectLedgerPurpose::Forward);
    side_effect_binding_for(
        ctx.run_id(),
        ctx.runtime_spec(),
        &ctx.node().node_id,
        ledger_purpose,
    )
}

fn side_effect_binding_for(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    node_id: &NodeId,
    ledger_purpose: events::SideEffectLedgerPurpose,
) -> Result<RunnerSideEffectBinding> {
    let pair_id = runtime_spec
        .side_effect_pair_for_submit_node(node_id)
        .cloned()
        .ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "side-effect node {node_id} is missing certified verify pair"
            ))
        })?;
    let ledger_key = side_effect_ledger_key(run_id, &pair_id, &ledger_purpose)?;
    Ok(RunnerSideEffectBinding {
        ledger_key,
        ledger_purpose,
        pair_id,
        invocation_epoch: 1,
    })
}

fn pre_invocation_lane_claim(
    ctx: &PreInvocationRunCtx<'_>,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> Result<(RunnerSideEffectBinding, RuntimeSideEffectClaimAuthority)> {
    let view = SideEffectAttemptView::from_pre_invocation_context(ctx)?;
    match view.phase() {
        None => {
            let ledger_purpose = terminal_forward_pair_for_remediation(
                ctx.runtime_spec(),
                ctx.projections(),
                ctx.node(),
            )?
            .map(|forward_pair_id| events::SideEffectLedgerPurpose::Remediation { forward_pair_id })
            .unwrap_or(events::SideEffectLedgerPurpose::Forward);
            let side_effect = side_effect_binding_for(
                ctx.run_id(),
                ctx.runtime_spec(),
                &ctx.node().node_id,
                ledger_purpose,
            )?;
            let claim = claim_authority_for(
                ctx.run_id(),
                &ctx.node().node_id,
                ctx.attempt_id(),
                &side_effect,
                1,
                resource_key,
            )?;
            Ok((side_effect, claim))
        }
        Some(store::SideEffectLedgerPhase::SubmissionKnown {
            claim,
            status: store::SideEffectSubmissionState::NotSubmitted,
        }) => {
            let _ = claim;
            Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} cannot retry after not-submitted proof",
                ctx.node().node_id
            )))
        }
        Some(phase) => Err(RuntimeError::InvalidRunnerOutput(format!(
            "exclusive side-effect node {} cannot preclaim a resource lane from {} phase",
            ctx.node().node_id,
            side_effect_ledger_phase_name(phase)
        ))),
    }
}

fn terminal_forward_pair_for_remediation(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
) -> Result<Option<SideEffectPairId>> {
    let Some(forward_node_id) = runtime_spec.forward_node_for_remediation(&node.node_id) else {
        return Ok(None);
    };
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == *forward_node_id
            && matches!(
                &projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
            && terminal_policies
                .require(&projection.pair_id)?
                .is_terminal_phase(&projection.phase)
        {
            return Ok(Some(projection.pair_id.clone()));
        }
    }
    Ok(None)
}

fn side_effect_ledger_key(
    run_id: &mfm_ids::RunId,
    pair_id: &mfm_ids::SideEffectPairId,
    ledger_purpose: &events::SideEffectLedgerPurpose,
) -> Result<events::SideEffectLedgerKey> {
    let digest = crate::content_digest_json(serde_json::json!({
        "ledger_purpose": ledger_purpose_key(ledger_purpose),
        "pair_id": pair_id.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    Ok(events::SideEffectLedgerKey::new(format!(
        "mfm.runtime.side_effect.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

fn claim_authority_for(
    run_id: &RunId,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> Result<RuntimeSideEffectClaimAuthority> {
    let claim_owner = claim_owner_for(run_id, node_id, attempt_id, side_effect, claim_generation)?;
    let claim_fencing_token =
        claim_fencing_token_for(node_id, attempt_id, side_effect, claim_generation)?;
    Ok(RuntimeSideEffectClaimAuthority {
        claim_owner,
        claim_generation,
        claim_fencing_token,
        resource_key,
    })
}

fn claim_owner_for(
    run_id: &RunId,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
) -> Result<events::RunnerInvocationId> {
    let digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    Ok(events::RunnerInvocationId::new(format!(
        "mfm.runtime.owner.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

fn claim_fencing_token_for(
    node_id: &NodeId,
    attempt_id: &AttemptId,
    side_effect: &RunnerSideEffectBinding,
    claim_generation: u32,
) -> Result<side_effect::ClaimFencingToken> {
    let digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": node_id.as_str(),
        "purpose": "side-effect-claim-fencing",
    }))?;
    Ok(side_effect::ClaimFencingToken::new(format!(
        "mfm.runtime.token.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

fn pre_invocation_resource_lane_requirement_digest(
    ctx: &PreInvocationRunCtx<'_>,
) -> Result<ContentDigest> {
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
        events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id, ..
        } => serde_json::json!({
            "forward_pair_id": forward_pair_id.as_str(),
            "kind": "remediation",
        }),
    }
}

fn canonical_mfm_value<T>(value: &T) -> Result<mfm_canonical::PlainCanonicalJsonBytes>
where
    T: MfmValue,
{
    let value = serde_json::to_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    canonical_json(value)
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

fn missing_driver_projection(label: &str) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!(
        "side-effect driver missing verified {label} projection"
    ))
}

fn build_side_effect_failed_with_release(
    ctx: &ErasedRunCtx<'_>,
    side_effect: RunnerSideEffectBinding,
    failure_phase: side_effect::FailurePhase,
    retryable: bool,
    safe_message: &'static str,
) -> Result<ErasedRunnerOutput> {
    let payloads = RunnerPayloadBuilder::new(ctx);
    let mut runner_output = RunnerOutputBuilder::new(ctx);
    if let Some(release) = SideEffectEvidenceBuilder::new(ctx)
        .resource_lane_released_payload(side_effect.clone(), "side_effect.failed")?
    {
        runner_output.payload(release);
    }
    let error = events::MfmErrorInfo::new(
        events::ErrorCode::new("side_effect_verification_failed")?,
        events::ErrorCategory::SideEffect,
        retryable,
        safe_message,
    )?;
    runner_output.payload(payloads.side_effect_failed(
        side_effect,
        events::SideEffectPairRole::Verify,
        failure_phase,
        retryable,
        error,
    ));
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
        start_claim: Option<RunnerClaimBinding>,
        proof: &Proof,
    ) -> Result<ErasedRunnerOutput>
    where
        Proof: MfmValue,
    {
        self.not_submitted_proven_with_role(
            side_effect,
            events::SideEffectPairRole::Submit,
            start_claim,
            proof,
        )
    }

    fn not_submitted_proven_with_role<Proof>(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        start_claim: Option<RunnerClaimBinding>,
        proof: &Proof,
    ) -> Result<ErasedRunnerOutput>
    where
        Proof: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.not_submitted_proof(proof)?;
        self.terminal_single_artifact_output(
            side_effect,
            start_claim,
            &artifact,
            "side_effect.not_submitted",
            |payloads, side_effect, artifact| {
                payloads.side_effect_not_submitted_proven_with_role(
                    side_effect,
                    pair_role,
                    artifact,
                )
            },
        )
    }

    /// Builds submission observed evidence.
    pub fn submission_observed<Submission>(
        &self,
        side_effect: RunnerSideEffectBinding,
        start_claim: Option<RunnerClaimBinding>,
        submission: &Submission,
    ) -> Result<ErasedRunnerOutput>
    where
        Submission: MfmValue,
    {
        self.submission_observed_with_role(
            side_effect,
            events::SideEffectPairRole::Submit,
            start_claim,
            submission,
        )
    }

    fn submission_observed_with_role<Submission>(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        start_claim: Option<RunnerClaimBinding>,
        submission: &Submission,
    ) -> Result<ErasedRunnerOutput>
    where
        Submission: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission(submission)?;
        self.single_artifact_output_with_optional_start(
            side_effect,
            start_claim,
            &artifact,
            None,
            |payloads, side_effect, artifact| {
                payloads.side_effect_submission_observed_with_role(side_effect, pair_role, artifact)
            },
        )
    }

    /// Builds submission unknown evidence.
    pub fn submission_unknown<Evidence>(
        &self,
        side_effect: RunnerSideEffectBinding,
        start_claim: Option<RunnerClaimBinding>,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.submission_unknown(evidence)?;
        self.single_artifact_output_with_optional_start(
            side_effect,
            start_claim,
            &artifact,
            None,
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

    /// Builds terminal receipt-observed evidence and releases any active resource lane.
    pub fn receipt_observed_terminal<Receipt>(
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
        self.terminal_single_artifact_output(
            side_effect,
            None,
            &artifact,
            "side_effect.receipt_observed",
            |payloads, side_effect, artifact| {
                payloads.side_effect_receipt_observed(
                    side_effect,
                    artifact,
                    replay.replay_verifier_id,
                    replay.resource_touched_set,
                )
            },
        )
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
        pair_role: events::SideEffectPairRole,
        start_claim: Option<RunnerClaimBinding>,
        ambiguity_code: events::AmbiguityCode,
        evidence: &Evidence,
    ) -> Result<ErasedRunnerOutput>
    where
        Evidence: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let artifact = artifacts.ambiguity_evidence(evidence)?;
        self.single_artifact_output_with_optional_start(
            side_effect,
            start_claim,
            &artifact,
            None,
            |payloads, side_effect, artifact| {
                payloads.side_effect_ambiguous(side_effect, pair_role, ambiguity_code, artifact)
            },
        )
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
        output.stage_side_effect_runtime_evidence(&intent_artifact, &side_effect)?;
        if let Some(prepared_artifact) = &prepared_artifact {
            output.stage_side_effect_runtime_evidence(prepared_artifact, &side_effect)?;
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
        output.stage_side_effect_runtime_evidence(artifact, &side_effect)?;
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
        let holder = store::SideEffectPairLedgerRef::new(
            self.ctx.run_id().clone(),
            side_effect.pair_id.clone(),
        );
        let Some((_, lane)) = self
            .ctx
            .projections()
            .resource_lanes()
            .find(|(_, projection)| projection.holder == holder)
        else {
            return Ok(None);
        };
        if lane.holder.pair_id != side_effect.pair_id
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
