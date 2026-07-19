use super::builder_helpers::{
    artifact_schema_id, canonical_json, ensure_artifact_role, runtime_value_error,
};
use super::*;

/// Capability and adapter binding metadata for runner payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerCapabilityBinding {
    /// Capability kind used by the runner.
    pub(crate) capability_kind: CapabilityKind,
    /// Capability version used by the runner.
    pub(crate) capability_version: CapabilityVersion,
    /// Adapter kind used by the runner.
    pub(crate) adapter_kind: AdapterKind,
    /// Adapter version used by the runner.
    pub(crate) adapter_version: AdapterVersion,
}

impl RunnerCapabilityBinding {
    /// Builds binding metadata for a typed capability and adapter identity.
    pub fn for_capability<C>(
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    ) -> Result<Self>
    where
        C: CapabilitySpec,
    {
        Ok(Self {
            capability_kind: C::kind()
                .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?,
            capability_version: C::version()
                .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?,
            adapter_kind,
            adapter_version,
        })
    }

    /// Returns the capability kind used by the runner.
    pub const fn capability_kind(&self) -> &CapabilityKind {
        &self.capability_kind
    }

    /// Returns the capability version used by the runner.
    pub const fn capability_version(&self) -> &CapabilityVersion {
        &self.capability_version
    }

    /// Returns the adapter kind used by the runner.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Returns the adapter version used by the runner.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }
}

/// Shared side-effect ledger coordinates for runner payload builders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerSideEffectBinding {
    /// Side-effect ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: mfm_ids::SideEffectPairId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
}

/// Claim metadata for a side-effect invocation owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerClaimBinding {
    /// Claim owner.
    pub claim_owner: events::RunnerInvocationId,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Prepared invocation metadata for a side-effect runner payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunnerPreparedInvocationBinding {
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
    /// Optional exclusive resource lane key evidence echoed from the held lane.
    pub resource_key: Option<events::ResourceKeyEvidence>,
}

/// Builder for runner-owned event payloads.
pub struct RunnerPayloadBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
}

macro_rules! runner_side_effect_payload {
    (
        $builder:expr,
        $side_effect:expr,
        $pair_role:expr,
        $variant:ident {
            $($field:ident : $value:expr),* $(,)?
        }
    ) => {
        side_effect::$variant {
            spec_hash: $builder.ctx.spec_hash().clone(),
            node_id: $builder.ctx.node().node_id.clone(),
            attempt_id: $builder.ctx.attempt_id().clone(),
            ledger_key: $side_effect.ledger_key.clone(),
            ledger_purpose: $side_effect.ledger_purpose.clone(),
            pair_id: $side_effect.pair_id.clone(),
            pair_role: $pair_role,
            invocation_epoch: $side_effect.invocation_epoch,
            $($field: $value,)*
        }
    };
}

impl<'a, 'ctx> RunnerPayloadBuilder<'a, 'ctx> {
    /// Creates a payload builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Builds a `CellProduced` runner payload from a state-output artifact.
    pub fn cell_produced(&self, artifact: &RunnerJsonArtifact) -> Result<RunnerEventPayload> {
        ensure_artifact_role(artifact, events::ArtifactRole::StateOutput)?;
        Ok(RunnerEventPayload::CellProduced(events::CellProduced {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            cell_id: self.ctx.node().output_cell.clone(),
            scope_id: self.ctx.output_cell().scope_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            semantic_type_id: self.ctx.output_cell().semantic_type_id.clone(),
            schema_id: self.ctx.output_cell().schema_id.clone(),
            value_lineage: self.ctx.output_cell().value_lineage.clone(),
            context: self.ctx.output_cell().context.clone(),
            artifact_id: artifact.evidence.artifact_id.clone(),
            content_digest: artifact.evidence.digest.clone(),
            evidence_hash: artifact.evidence.evidence_hash()?,
            producer_state_kind: Some(self.ctx.node().state_kind.clone()),
            producer_state_version: Some(self.ctx.node().state_version.clone()),
        }))
    }

    /// Builds a `CellSkipped` runner payload for a certified maybe-skipped output cell.
    pub fn cell_skipped(&self, skip_reason: events::SkipReason) -> RunnerEventPayload {
        RunnerEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            cell_id: self.ctx.node().output_cell.clone(),
            scope_id: self.ctx.output_cell().scope_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            semantic_type_id: self.ctx.output_cell().semantic_type_id.clone(),
            schema_id: self.ctx.output_cell().schema_id.clone(),
            value_lineage: self.ctx.output_cell().value_lineage.clone(),
            context: self.ctx.output_cell().context.clone(),
            skip_reason,
        })
    }

    /// Builds a `SideEffectIntentPersisted` runner payload.
    pub(crate) fn side_effect_intent_persisted<Idempotency>(
        &self,
        side_effect: RunnerSideEffectBinding,
        intent: &RunnerJsonArtifact,
        idempotency: &Idempotency,
        idempotency_key: events::IdempotencyKeyRef,
        binding: RunnerCapabilityBinding,
    ) -> Result<RunnerEventPayload>
    where
        Idempotency: MfmValue,
    {
        ensure_artifact_role(intent, events::ArtifactRole::SideEffectIntent)?;
        Ok(RunnerEventPayload::SideEffectIntentPersisted(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                IntentPersisted {
                    scope_id: self.ctx.node().scope_id.clone(),
                    intent_schema_id: artifact_schema_id(intent)?,
                    intent_hash: intent.evidence.digest.clone(),
                    intent_artifact_id: intent.evidence.artifact_id.clone(),
                    intent_artifact_evidence_hash: intent.evidence.evidence_hash()?,
                    idempotency_input_schema_id: Idempotency::schema_id()
                        .map_err(runtime_value_error)?,
                    idempotency_input_hash: canonical_json(idempotency)?.content_digest(),
                    idempotency_key: idempotency_key,
                    capability_kind: binding.capability_kind,
                    capability_version: binding.capability_version,
                    adapter_kind: binding.adapter_kind,
                    adapter_version: binding.adapter_version,
                }
            ),
        ))
    }

    /// Builds a `SideEffectClaimed` runner payload.
    pub(crate) fn side_effect_claimed(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectClaimed(runner_side_effect_payload!(
            self,
            side_effect,
            events::SideEffectPairRole::Submit,
            Claimed {
                claim_owner: claim.claim_owner,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
            }
        ))
    }

    /// Builds a `ResourceLaneReleaseIntent` runner payload for terminal lane release.
    pub(crate) fn resource_lane_release_intent(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim_id: events::ResourceLaneClaimId,
        release_reason: events::ResourceLaneReleaseReason,
    ) -> RunnerEventPayload {
        RunnerEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
            spec_hash: self.ctx.spec_hash().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: side_effect.invocation_epoch,
            claim_id,
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason,
        })
    }

    /// Builds a `SideEffectInvocationPrepared` runner payload.
    pub(crate) fn side_effect_invocation_prepared(
        &self,
        side_effect: RunnerSideEffectBinding,
        prepared: &RunnerJsonArtifact,
        prepared_binding: RunnerPreparedInvocationBinding,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(prepared, events::ArtifactRole::PreparedInvocation)?;
        Ok(RunnerEventPayload::SideEffectInvocationPrepared(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                InvocationPrepared {
                    claim_generation: prepared_binding.claim_generation,
                    claim_fencing_token: prepared_binding.claim_fencing_token,
                    resource_key: prepared_binding.resource_key,
                    prepared_schema_id: artifact_schema_id(prepared)?,
                    prepared_artifact_id: prepared.evidence.artifact_id.clone(),
                    prepared_hash: prepared.evidence.digest.clone(),
                    prepared_artifact_evidence_hash: prepared.evidence.evidence_hash()?,
                }
            ),
        ))
    }

    /// Builds a `SideEffectInvocationStarted` runner payload.
    pub(crate) fn side_effect_invocation_started(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectInvocationStarted(runner_side_effect_payload!(
            self,
            side_effect,
            events::SideEffectPairRole::Submit,
            InvocationStarted {
                claim_owner: claim.claim_owner,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
            }
        ))
    }

    /// Builds a `SideEffectNotSubmittedProven` runner payload with an explicit pair role.
    pub(crate) fn side_effect_not_submitted_proven_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        proof: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(proof, events::ArtifactRole::NotSubmittedProof)?;
        Ok(RunnerEventPayload::SideEffectNotSubmittedProven(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                NotSubmittedProven {
                    proof_schema_id: artifact_schema_id(proof)?,
                    proof_hash: proof.evidence.digest.clone(),
                    proof_artifact_id: proof.evidence.artifact_id.clone(),
                    proof_artifact_evidence_hash: proof.evidence.evidence_hash()?,
                }
            ),
        ))
    }

    /// Builds a `SideEffectSubmissionObserved` runner payload with an explicit pair role.
    pub(crate) fn side_effect_submission_observed_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        submission: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(submission, events::ArtifactRole::Submission)?;
        Ok(RunnerEventPayload::SideEffectSubmissionObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                SubmissionObserved {
                    submission_schema_id: artifact_schema_id(submission)?,
                    submission_hash: submission.evidence.digest.clone(),
                    submission_artifact_id: submission.evidence.artifact_id.clone(),
                    submission_artifact_evidence_hash: submission.evidence.evidence_hash()?,
                }
            ),
        ))
    }

    /// Builds a `SideEffectSubmissionUnknown` runner payload.
    pub(crate) fn side_effect_submission_unknown(
        &self,
        side_effect: RunnerSideEffectBinding,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::SubmissionUnknownEvidence)?;
        Ok(RunnerEventPayload::SideEffectSubmissionUnknown(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                SubmissionUnknown {
                    evidence_schema_id: artifact_schema_id(evidence)?,
                    evidence_hash: evidence.evidence.digest.clone(),
                    evidence_artifact_id: evidence.evidence.artifact_id.clone(),
                    evidence_artifact_evidence_hash: evidence.evidence.evidence_hash()?,
                }
            ),
        ))
    }

    /// Builds a `SideEffectReceiptObserved` runner payload.
    pub(crate) fn side_effect_receipt_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        receipt: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(receipt, events::ArtifactRole::Receipt)?;
        Ok(RunnerEventPayload::SideEffectReceiptObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Verify,
                ReceiptObserved {
                    receipt_schema_id: artifact_schema_id(receipt)?,
                    receipt_hash: receipt.evidence.digest.clone(),
                    receipt_artifact_id: receipt.evidence.artifact_id.clone(),
                    receipt_artifact_evidence_hash: receipt.evidence.evidence_hash()?,
                    replay_verifier_id: replay_verifier_id,
                    resource_touched_set: resource_touched_set,
                }
            ),
        ))
    }

    /// Builds a `SideEffectConfirmationObserved` runner payload.
    pub(crate) fn side_effect_confirmation_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        confirmation: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(confirmation, events::ArtifactRole::Confirmation)?;
        Ok(RunnerEventPayload::SideEffectConfirmationObserved(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Verify,
                ConfirmationObserved {
                    confirmation_schema_id: artifact_schema_id(confirmation)?,
                    confirmation_hash: confirmation.evidence.digest.clone(),
                    confirmation_artifact_id: confirmation.evidence.artifact_id.clone(),
                    confirmation_artifact_evidence_hash: confirmation.evidence.evidence_hash()?,
                    replay_verifier_id: replay_verifier_id,
                    resource_touched_set: resource_touched_set,
                }
            ),
        ))
    }

    /// Builds a `SideEffectAmbiguous` runner payload.
    pub(crate) fn side_effect_ambiguous(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        ambiguity_code: events::AmbiguityCode,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::AmbiguityEvidence)?;
        Ok(RunnerEventPayload::SideEffectAmbiguous(
            runner_side_effect_payload!(
                self,
                side_effect,
                pair_role,
                Ambiguous {
                    ambiguity_code: ambiguity_code,
                    evidence_schema_id: artifact_schema_id(evidence)?,
                    evidence_hash: evidence.evidence.digest.clone(),
                    evidence_artifact_id: evidence.evidence.artifact_id.clone(),
                    evidence_artifact_evidence_hash: evidence.evidence.evidence_hash()?,
                }
            ),
        ))
    }

    /// Builds a `SideEffectFailed` runner payload.
    pub(crate) fn side_effect_failed(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        failure_phase: side_effect::FailurePhase,
        retryable: bool,
        error: events::MfmErrorInfo,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectFailed(runner_side_effect_payload!(
            self,
            side_effect,
            pair_role,
            Failed {
                failure_phase: failure_phase,
                retryable: retryable,
                error: error,
            }
        ))
    }
}
