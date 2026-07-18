use std::future::Future;
use std::pin::Pin;

use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    short_stable_id_fragment, ArtifactId, AttemptId, ContentDigest, NodeId, RunId, SideEffectPairId,
};
use mfm_program::SideEffectIntent;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmValue;

use crate::runner_kit::{
    RunnerClaimBinding, RunnerPreparedInvocationBinding, RunnerSideEffectBinding,
};
use crate::side_effect_lifecycle::SideEffectAttemptView;
use crate::{
    canonical_json, CertifiedRuntimeSpec, ErasedRunCtx, ErasedRunnerOutput, MaterializedInputs,
    PreInvocationRunCtx, Result, RunnerArtifactBuilder, RunnerCapabilityBinding,
    RunnerEventPayload, RunnerJsonArtifact, RunnerOutputBuilder, RunnerOutputSettlement,
    RunnerPayloadBuilder, RuntimeError, StagedArtifact, StagedRetentionRefs,
};

/// Boxed future returned by side-effect driver callbacks.
pub type SideEffectDriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Future returned by callbacks that submit or recover a prepared side-effect invocation.
pub type SideEffectSubmissionDecisionFuture<'a, Submission, RecoveryEvidence> =
    SideEffectDriverFuture<'a, SideEffectSubmissionDecision<Submission, RecoveryEvidence>>;

/// Future returned by callbacks that recover a previously unknown submission.
pub type SideEffectUnknownSubmissionDecisionFuture<'a, Submission, RecoveryEvidence> =
    SideEffectDriverFuture<'a, SideEffectUnknownSubmissionDecision<Submission, RecoveryEvidence>>;

/// Public prepared invocation plus optional process-local authority awaiting durable settlement.
pub struct SideEffectPreparedInvocation<T> {
    evidence: T,
    settlement: Option<RunnerOutputSettlement>,
}

impl<T> SideEffectPreparedInvocation<T> {
    /// Creates prepared evidence that carries no process-local settlement.
    pub const fn new(evidence: T) -> Self {
        Self {
            evidence,
            settlement: None,
        }
    }

    /// Creates prepared evidence with authority promoted only after its append succeeds.
    pub fn with_settlement(evidence: T, settlement: RunnerOutputSettlement) -> Self {
        Self {
            evidence,
            settlement: Some(settlement),
        }
    }

    /// Separates persisted evidence from its optional process-local settlement.
    pub fn into_parts(self) -> (T, Option<RunnerOutputSettlement>) {
        (self.evidence, self.settlement)
    }
}

/// Builds pre-invocation side-effect resource-lane claim evidence for the first epoch.
pub fn preclaim_side_effect_resource_lane<Intent, Idempotency>(
    ctx: &PreInvocationRunCtx<'_>,
    intent: &Intent,
    idempotency: &Idempotency,
    capability_binding: RunnerCapabilityBinding,
    resource_key: events::ResourceKeyEvidence,
) -> Result<ErasedRunnerOutput>
where
    Intent: MfmValue,
    Idempotency: MfmValue,
{
    let (side_effect, claim) = pre_invocation_lane_claim(ctx, Some(resource_key.clone()))?;
    let intent_artifact =
        pre_invocation_value_artifact(ctx, intent, events::ArtifactRole::SideEffectIntent)?;
    let staged_artifact = StagedArtifact::inline_pre_invocation_side_effect_artifact(
        ctx,
        intent_artifact.bytes.clone(),
        intent_artifact.evidence.clone(),
        side_effect.ledger_key.clone(),
        side_effect.invocation_epoch,
    )?;
    Ok(ErasedRunnerOutput::from_parts(
        vec![staged_artifact],
        vec![StagedRetentionRefs::runtime_evidence(vec![intent_artifact
            .evidence
            .retention_ref()?])],
        vec![
            pre_invocation_side_effect_intent_persisted(
                ctx,
                side_effect.clone(),
                &intent_artifact,
                idempotency,
                side_effect_idempotency_key(idempotency)?,
                capability_binding,
            )?,
            RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: events::SideEffectPairRole::Submit,
                claim_owner: claim.claim_owner.clone(),
                invocation_epoch: side_effect.invocation_epoch,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token.clone(),
            }),
            pre_invocation_resource_lane_claim_intent(ctx, side_effect, resource_key)?,
        ],
    ))
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
            intent_artifact_evidence_hash: intent.evidence.evidence_hash()?,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SideEffectStep {
    /// No ledger evidence exists yet; persist intent and claim the first invocation epoch.
    Claim,
    /// A committed side-effect claim can now prepare immutable invocation authority.
    Prepare {
        /// Invocation epoch to prepare and start.
        invocation_epoch: u32,
    },
    /// A started invocation can submit exact prepared authority.
    Submit {
        /// Invocation epoch to submit.
        invocation_epoch: u32,
    },
    /// A durable submission boundary result exists; complete the submit anchor.
    CompleteSubmissionBoundary {
        /// Invocation epoch whose boundary result was recorded.
        invocation_epoch: u32,
    },
}

impl SideEffectStep {
    /// Selects the protocol action for a verified side-effect attempt view.
    pub(crate) fn from_attempt_view(view: &SideEffectAttemptView<'_>) -> Result<Self> {
        let Some(phase) = view.phase() else {
            return Ok(Self::Claim);
        };
        match phase {
            store::SideEffectLedgerPhase::Claimed { claim } => Ok(Self::Prepare {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::Started { claim, .. } => Ok(Self::Submit {
                invocation_epoch: claim.invocation_epoch,
            }),
            store::SideEffectLedgerPhase::SubmissionKnown { claim, .. } => {
                Ok(Self::CompleteSubmissionBoundary {
                    invocation_epoch: claim.invocation_epoch,
                })
            }
            store::SideEffectLedgerPhase::IntentPersisted { .. }
            | store::SideEffectLedgerPhase::Prepared { .. }
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
    replay_verifier_id: events::ReplayVerifierId,
    /// Optional exact touched resource set evidence.
    resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
}

impl SideEffectReplayEvidence {
    /// Creates replay verifier evidence for observed side-effect material.
    pub fn new(
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Self {
        Self {
            replay_verifier_id,
            resource_touched_set,
        }
    }
}

/// Input for persisting authored intent and claiming a side-effect invocation.
pub(crate) struct SideEffectClaimEvidence<'a, Intent, Idempotency> {
    /// Side-effect ledger coordinates.
    pub(crate) side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub(crate) claim: RuntimeSideEffectClaimAuthority,
    /// Typed side-effect intent evidence.
    pub(crate) intent: &'a Intent,
    /// Typed idempotency input evidence.
    pub(crate) idempotency: &'a Idempotency,
    /// Capability implementation that will perform the mutation.
    pub(crate) capability_binding: RunnerCapabilityBinding,
}

/// Kernel-owned authored side-effect authority.
pub(crate) struct AuthoredSideEffect<Intent, Idempotency> {
    /// Typed intent evidence.
    pub(crate) intent: Intent,
    /// Typed idempotency input evidence.
    pub(crate) idempotency: Idempotency,
    /// Stable schema-bound idempotency key derived by the kernel.
    pub(crate) idempotency_key: events::IdempotencyKeyRef,
    /// Capability implementation that will perform the mutation.
    pub(crate) capability_binding: RunnerCapabilityBinding,
}

impl<Intent, Idempotency> AuthoredSideEffect<Intent, Idempotency> {
    fn new(
        intent: Intent,
        idempotency: Idempotency,
        capability_binding: RunnerCapabilityBinding,
    ) -> Result<Self>
    where
        Idempotency: MfmValue,
    {
        let idempotency_key = side_effect_idempotency_key(&idempotency)?;
        Ok(Self {
            intent,
            idempotency,
            idempotency_key,
            capability_binding,
        })
    }
}

/// Submission recovery result returned by side-effect driver callbacks.
pub enum SideEffectSubmissionDecision<Submission, RecoveryEvidence> {
    /// Submission was observed and can be persisted.
    Observed(Submission),
    /// Submission status could not be determined and uncertainty evidence can be persisted.
    Unknown(RecoveryEvidence),
    /// The invocation was proven not submitted.
    NotSubmitted(RecoveryEvidence),
    /// Submission recovery became ambiguous and must stop emitting further evidence.
    Ambiguous {
        /// Ambiguity classifier code.
        ambiguity_code: events::AmbiguityCode,
        /// Redaction-safe ambiguity evidence.
        evidence: RecoveryEvidence,
    },
}

/// Verify-side recovery result for a previously unknown submission.
pub enum SideEffectUnknownSubmissionDecision<Submission, RecoveryEvidence> {
    /// Submission was observed and can be persisted by the verify role.
    Observed(Submission),
    /// Submission status remains unknown; the scheduler must stop without committing evidence.
    StillUnknown,
    /// The invocation was proven not submitted by the verify role.
    NotSubmitted(RecoveryEvidence),
    /// Recovery became ambiguous and must stop emitting further evidence.
    Ambiguous {
        /// Ambiguity classifier code.
        ambiguity_code: events::AmbiguityCode,
        /// Redaction-safe ambiguity evidence.
        evidence: RecoveryEvidence,
    },
}

/// Observed side-effect evidence plus replay verifier metadata.
pub struct SideEffectObservedEvidence<T> {
    /// Typed observed evidence.
    evidence: T,
    /// Replay verifier metadata for this observation.
    replay: SideEffectReplayEvidence,
}

impl<T> SideEffectObservedEvidence<T> {
    /// Creates observed side-effect evidence with replay verifier metadata.
    pub fn new(evidence: T, replay: SideEffectReplayEvidence) -> Self {
        Self { evidence, replay }
    }
}

/// One adapter contract for preparing, submitting, recovering, and verifying a side effect.
///
/// Implementations own artifact reads, live providers, signing, and domain reconstruction. The
/// runtime owns ledger phase selection, idempotency, and evidence emission.
pub trait SideEffectAdapter {
    /// Typed side-effect intent evidence.
    type Intent: MfmValue + Send + Sync + 'static;
    /// Typed idempotency input evidence.
    type Idempotency: MfmValue + Send + Sync + 'static;
    /// Public prepared invocation evidence.
    type PreparedInvocation: MfmValue + Send + Sync + 'static;
    /// Typed submission evidence.
    type Submission: MfmValue + Send + Sync + 'static;
    /// Typed recovery, uncertainty, and ambiguity evidence.
    type RecoveryEvidence: MfmValue + Send + Sync + 'static;
    /// Typed receipt evidence.
    type Receipt: MfmValue + Send + Sync + 'static;
    /// Typed confirmation evidence.
    type Confirmation: MfmValue + Send + Sync + 'static;
    /// Typed terminal state output.
    type Output: MfmValue + Send + Sync + 'static;

    /// Returns the fixed certified capability and adapter binding for this runner.
    fn capability_binding(&self) -> Result<RunnerCapabilityBinding>;

    /// Invokes the state-owned pure intent reducer for the submit node.
    fn authored_intent<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
    ) -> SideEffectDriverFuture<'a, SideEffectIntent<Self::Intent, Self::Idempotency>>;

    /// Prepares required public invocation authority under the committed claim.
    fn prepare<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        intent: &'a Self::Intent,
        idempotency: &'a Self::Idempotency,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocation<Self::PreparedInvocation>>;

    /// Loads and type-checks retained prepared invocation authority.
    fn load_prepared<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation>;

    /// Submits exact prepared authority; implementations must be restart-safe.
    fn submit_prepared<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectSubmissionDecisionFuture<'a, Self::Submission, Self::RecoveryEvidence>;

    /// Recovers a previously unknown submission without crossing the submission boundary again.
    fn recover_unknown<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectUnknownSubmissionDecisionFuture<'a, Self::Submission, Self::RecoveryEvidence>;

    /// Reads receipt evidence for an observed submission.
    fn observe_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>>;

    /// Builds confirmation evidence from a stored receipt.
    fn observe_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>>;

    /// Maps stored receipt evidence to the verified state output.
    fn output_from_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output>;

    /// Maps stored confirmation evidence to the verified state output.
    fn output_from_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output>;
}

/// Generic one-step side-effect submit driver.
pub struct SideEffectDriver;

#[path = "side_effect_driver/evidence.rs"]
mod evidence;
pub(crate) use self::evidence::SideEffectEvidenceBuilder;

#[path = "side_effect_driver/driver.rs"]
mod driver;
pub(crate) use self::driver::{authored_plan, verify_authored_plan};

/// Generic one-step side-effect verification driver.
pub struct SideEffectVerifyDriver;

#[path = "side_effect_driver/verify.rs"]
mod verify;

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
            status: store::SideEffectSubmissionState::NotSubmitted,
            ..
        }) => Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} cannot retry after not-submitted proof",
            ctx.node().node_id
        ))),
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
    let claim_owner_digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    let claim_owner = events::RunnerInvocationId::new(format!(
        "mfm.runtime.owner.{}",
        short_stable_id_fragment(claim_owner_digest.as_str(), 32)
    ))?;
    let claim_fencing_token_digest = crate::content_digest_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "claim_generation": claim_generation,
        "ledger_key": side_effect.ledger_key.as_str(),
        "node_id": node_id.as_str(),
        "purpose": "side-effect-claim-fencing",
    }))?;
    let claim_fencing_token = side_effect::ClaimFencingToken::new(format!(
        "mfm.runtime.token.{}",
        short_stable_id_fragment(claim_fencing_token_digest.as_str(), 32)
    ))?;
    Ok(RuntimeSideEffectClaimAuthority {
        claim_owner,
        claim_generation,
        claim_fencing_token,
        resource_key,
    })
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

/// Derives the canonical runtime idempotency key for typed side-effect input.
///
/// Domain replay verifiers use this helper to recompute the exact key authored by the live
/// side-effect driver rather than maintaining a second hashing formula.
pub fn side_effect_idempotency_key<T>(value: &T) -> Result<events::IdempotencyKeyRef>
where
    T: MfmValue,
{
    let schema_id =
        T::schema_id().map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let semantic_type_id =
        T::semantic_id().map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let value = serde_json::to_value(value)
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let digest = crate::content_digest_json(serde_json::json!({
        "schema_id": schema_id.as_str(),
        "semantic_type_id": semantic_type_id.as_str(),
        "value": value,
    }))?;
    Ok(events::IdempotencyKeyRef::new(format!(
        "mfm.runtime.idempotency.{}",
        digest.as_str()
    ))?)
}

fn missing_driver_projection(label: &str) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!(
        "side-effect driver missing verified {label} projection"
    ))
}
