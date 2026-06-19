use mfm_events::v1::{self as events, side_effect};
use mfm_values::MfmValue;
use serde::Serialize;

use crate::{
    ErasedRunCtx, ErasedRunnerOutput, Result, RunnerArtifactBuilder, RunnerCapabilityBinding,
    RunnerClaimBinding, RunnerJsonArtifact, RunnerOutputBuilder, RunnerPayloadBuilder,
    RunnerPreparedInvocationBinding, RunnerSideEffectBinding,
};

/// Claim authority used when building side-effect evidence for one invocation epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectClaimAuthority {
    /// Runner invocation that owns the claim.
    pub claim_owner: events::RunnerInvocationId,
    /// Claim generation for this invocation epoch.
    pub claim_generation: u32,
    /// Fencing token bound to the claim generation.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
    /// Optional exclusive resource lane key evidence.
    pub resource_key: Option<events::ResourceKeyEvidence>,
}

impl SideEffectClaimAuthority {
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
    pub side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub claim: SideEffectClaimAuthority,
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
    pub side_effect: RunnerSideEffectBinding,
    /// Claim authority for the invocation epoch.
    pub claim: SideEffectClaimAuthority,
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
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_not_submitted_proven(side_effect, artifact)
        })
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
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_confirmation_observed(
                side_effect,
                artifact,
                replay.replay_verifier_id,
                replay.resource_touched_set,
            )
        })
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
        self.single_artifact_output(side_effect, &artifact, |payloads, side_effect, artifact| {
            payloads.side_effect_ambiguous(side_effect, ambiguity_code, artifact)
        })
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
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.stage_side_effect_artifact(
            artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        output.retain_runtime_evidence(artifact);
        output.payload(payload(&payloads, side_effect, artifact)?);
        Ok(output.finish())
    }
}
