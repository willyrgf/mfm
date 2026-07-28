use super::*;

/// Builder for side-effect runner evidence outputs.
///
/// This builder stages artifacts and emits runner payloads only. It does not derive commit
/// preconditions, append commits, perform live IO, read prior artifacts, or create attempt
/// failures.
pub(crate) struct SideEffectEvidenceBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
}

impl<'a, 'ctx> SideEffectEvidenceBuilder<'a, 'ctx> {
    /// Creates a side-effect evidence builder for one runner invocation context.
    pub(crate) fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Builds prepared invocation and started evidence for an already claimed lane.
    pub(crate) fn prepare_claimed_invocation_and_start<Prepared>(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RuntimeSideEffectClaimAuthority,
        prepared_invocation: &Prepared,
    ) -> Result<ErasedRunnerOutput>
    where
        Prepared: MfmValue,
    {
        let artifacts = RunnerArtifactBuilder::new(self.ctx);
        let payloads = RunnerPayloadBuilder::new(self.ctx);
        let prepared_artifact = artifacts.prepared_invocation(prepared_invocation)?;
        let mut output = RunnerOutputBuilder::new(self.ctx);
        output.stage_side_effect_artifact(
            &prepared_artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        let prepared_key = (
            prepared_artifact.evidence().artifact_id.clone(),
            prepared_artifact.evidence().evidence_hash()?,
        );
        let retained = self.ctx.lifecycle().retention().is_some_and(|retention| {
            matches!(
                retention.visit_references(|reference| {
                    if reference.artifact_id == prepared_key.0
                        && reference.evidence_hash == prepared_key.1
                    {
                        std::ops::ControlFlow::Break(())
                    } else {
                        std::ops::ControlFlow::Continue(())
                    }
                }),
                std::ops::ControlFlow::Break(())
            )
        });
        if !retained {
            output.retain_runtime_evidence(&prepared_artifact)?;
        }
        output.payload(payloads.side_effect_invocation_prepared(
            side_effect.clone(),
            &prepared_artifact,
            claim.prepared_binding(),
        )?);
        output.payload(payloads.side_effect_invocation_started(side_effect, claim.claim_binding()));
        Ok(output.finish())
    }

    /// Persists authored intent and claims the first invocation epoch.
    pub(crate) fn claim_intent<Intent, Idempotency>(
        &self,
        evidence: SideEffectClaimEvidence<'_, Intent, Idempotency>,
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
        output.payload(payloads.side_effect_intent_persisted(
            side_effect.clone(),
            &intent_artifact,
            evidence.idempotency,
            side_effect_idempotency_key(evidence.idempotency)?,
            evidence.capability_binding,
        )?);
        output.payload(payloads.side_effect_claimed(side_effect, claim.claim_binding()));
        Ok(output.finish())
    }

    /// Builds not-submitted proof evidence.
    pub(crate) fn not_submitted_proven_with_role<Proof>(
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
        self.single_artifact_output(
            side_effect,
            start_claim,
            &artifact,
            Some("side_effect.not_submitted"),
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
    pub(crate) fn submission_observed_with_role<Submission>(
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
        self.single_artifact_output(
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
    pub(crate) fn submission_unknown<Evidence>(
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
        self.single_artifact_output(
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
    pub(crate) fn receipt_observed<Receipt>(
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
        self.single_artifact_output(
            side_effect,
            None,
            &artifact,
            None,
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

    /// Builds terminal receipt-observed evidence and releases any active resource lane.
    pub(crate) fn receipt_observed_terminal<Receipt>(
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
        self.single_artifact_output(
            side_effect,
            None,
            &artifact,
            Some("side_effect.receipt_observed"),
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
    pub(crate) fn confirmation_observed<Confirmation>(
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
        self.single_artifact_output(
            side_effect,
            None,
            &artifact,
            Some("side_effect.confirmed"),
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
    pub(crate) fn ambiguous<Evidence>(
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
        self.single_artifact_output(
            side_effect,
            start_claim,
            &artifact,
            None,
            |payloads, side_effect, artifact| {
                payloads.side_effect_ambiguous(side_effect, pair_role, ambiguity_code, artifact)
            },
        )
    }

    fn single_artifact_output<F>(
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

    pub(super) fn resource_lane_released_payload(
        &self,
        side_effect: RunnerSideEffectBinding,
        release_reason: &'static str,
    ) -> Result<Option<crate::RunnerEventPayload>> {
        let mut lane = None;
        let _ = self.ctx.lifecycle().visit_resource_lanes(|candidate| {
            let holder = candidate.holder();
            if holder.run_id() == self.ctx.run_id() && holder.pair_id() == &side_effect.pair_id {
                lane = Some(candidate);
                return std::ops::ControlFlow::Break(());
            }
            std::ops::ControlFlow::Continue(())
        });
        let Some(lane) = lane else {
            return Ok(None);
        };
        if lane.holder().pair_id() != &side_effect.pair_id
            || lane.ledger_purpose() != &side_effect.ledger_purpose
            || lane.invocation_epoch() != side_effect.invocation_epoch
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
            lane.claim_id().clone(),
            release_reason,
        )))
    }
}
