use super::*;

/// Builder for runner-owned JSON artifacts and staging handles.
pub struct RunnerArtifactBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
}

impl<'a, 'ctx> RunnerArtifactBuilder<'a, 'ctx> {
    /// Creates a builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self { ctx }
    }

    /// Canonicalizes a serializable value as plain JSON bytes.
    pub fn canonical_bytes<T>(&self, value: &T) -> Result<PlainCanonicalJsonBytes>
    where
        T: Serialize,
    {
        canonical_json(value)
    }

    /// Computes the canonical content digest for a serializable value.
    pub fn content_digest<T>(&self, value: &T) -> Result<ContentDigest>
    where
        T: Serialize,
    {
        Ok(self.canonical_bytes(value)?.content_digest())
    }

    /// Builds a state-output artifact using the value type metadata.
    pub fn state_output<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.value_artifact(value, events::ArtifactRole::StateOutput)
    }

    /// Builds a read fact-response artifact.
    pub fn fact_response<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::FactResponse)
    }

    /// Builds retained request/response evidence for an external capability read.
    pub fn external_read_evidence<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::ExternalReadEvidence)
    }

    /// Builds a private fact query replay evidence artifact from canonical facts-kernel bytes.
    pub fn fact_query_evidence(
        &self,
        evidence: &mfm_facts::FactQueryEvidence,
    ) -> Result<RunnerJsonArtifact> {
        let bytes =
            mfm_facts::canonical_fact_query_evidence_bytes(evidence).map_err(runtime_fact_error)?;
        self.json_bytes_artifact(
            bytes.as_bytes(),
            events::ArtifactRole::FactQueryEvidence,
            Some(mfm_facts::fact_query_evidence_schema_id().map_err(runtime_fact_error)?),
            None,
        )
    }

    /// Builds a side-effect intent artifact.
    pub(crate) fn side_effect_intent<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SideEffectIntent)
    }

    /// Builds a schema-less prepared invocation artifact.
    pub(crate) fn prepared_invocation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: Serialize,
    {
        self.json_artifact(value, events::ArtifactRole::PreparedInvocation, None, None)
    }

    /// Builds a not-submitted proof artifact.
    pub(crate) fn not_submitted_proof<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::NotSubmittedProof)
    }

    /// Builds a submission artifact.
    pub(crate) fn submission<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Submission)
    }

    /// Builds a submission-unknown evidence artifact.
    pub(crate) fn submission_unknown<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SubmissionUnknownEvidence)
    }

    /// Builds a receipt artifact.
    pub(crate) fn receipt<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Receipt)
    }

    /// Builds a confirmation artifact.
    pub(crate) fn confirmation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Confirmation)
    }

    /// Builds an ambiguity evidence artifact.
    pub(crate) fn ambiguity_evidence<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::AmbiguityEvidence)
    }

    /// Stages an inline attempt artifact through the existing runtime artifact boundary.
    pub fn staged_attempt(&self, artifact: &RunnerJsonArtifact) -> Result<StagedArtifact> {
        StagedArtifact::inline_attempt_artifact(
            self.ctx,
            artifact.bytes.clone(),
            artifact.evidence.clone(),
        )
    }

    /// Stages an inline side-effect artifact for one ledger epoch.
    pub(crate) fn staged_side_effect(
        &self,
        artifact: &RunnerJsonArtifact,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<StagedArtifact> {
        StagedArtifact::inline_side_effect_artifact(
            self.ctx,
            artifact.bytes.clone(),
            artifact.evidence.clone(),
            ledger_key,
            invocation_epoch,
        )
    }

    fn value_artifact<T>(&self, value: &T, role: events::ArtifactRole) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.json_artifact(
            value,
            role,
            Some(T::schema_id().map_err(runtime_value_error)?),
            Some(T::semantic_id().map_err(runtime_value_error)?),
        )
    }

    fn evidence_artifact<T>(
        &self,
        value: &T,
        role: events::ArtifactRole,
    ) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.json_artifact(
            value,
            role,
            Some(T::schema_id().map_err(runtime_value_error)?),
            None,
        )
    }

    fn json_artifact<T>(
        &self,
        value: &T,
        role: events::ArtifactRole,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<mfm_ids::SemanticTypeId>,
    ) -> Result<RunnerJsonArtifact>
    where
        T: Serialize,
    {
        let bytes = self.canonical_bytes(value)?;
        self.json_bytes_artifact(bytes.as_bytes(), role, schema_id, semantic_type_id)
    }

    fn json_bytes_artifact(
        &self,
        bytes: &[u8],
        role: events::ArtifactRole,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<mfm_ids::SemanticTypeId>,
    ) -> Result<RunnerJsonArtifact> {
        let digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: bytes.len() as u64,
            media_type: spec::MediaType::new("application/json")?,
            schema_id,
            semantic_type_id,
            producer_node_id: Some(self.ctx.node().node_id.clone()),
            producer_seed_id: None,
            artifact_role: role,
        };
        Ok(RunnerJsonArtifact {
            bytes: bytes.to_owned(),
            evidence,
        })
    }
}

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
        prepared: Option<&RunnerJsonArtifact>,
        prepared_binding: RunnerPreparedInvocationBinding,
    ) -> Result<RunnerEventPayload> {
        if let Some(artifact) = prepared {
            ensure_artifact_role(artifact, events::ArtifactRole::PreparedInvocation)?;
        }
        Ok(RunnerEventPayload::SideEffectInvocationPrepared(
            runner_side_effect_payload!(
                self,
                side_effect,
                events::SideEffectPairRole::Submit,
                InvocationPrepared {
                    claim_generation: prepared_binding.claim_generation,
                    claim_fencing_token: prepared_binding.claim_fencing_token,
                    resource_key: prepared_binding.resource_key,
                    prepared_artifact_id: prepared
                        .map(|artifact| artifact.evidence.artifact_id.clone()),
                    prepared_hash: prepared.map(|artifact| artifact.evidence.digest.clone()),
                    prepared_artifact_evidence_hash: prepared
                        .map(|artifact| artifact.evidence.evidence_hash())
                        .transpose()?,
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

/// Builder for assembling an erased runner output batch.
pub struct RunnerOutputBuilder<'a, 'ctx> {
    artifacts: RunnerArtifactBuilder<'a, 'ctx>,
    staged_artifacts: Vec<StagedArtifact>,
    staged_retention_refs: Vec<StagedRetentionRefs>,
    payloads: Vec<RunnerEventPayload>,
}

impl<'a, 'ctx> RunnerOutputBuilder<'a, 'ctx> {
    /// Creates an empty output builder bound to one runner invocation context.
    pub fn new(ctx: &'a ErasedRunCtx<'ctx>) -> Self {
        Self {
            artifacts: RunnerArtifactBuilder::new(ctx),
            staged_artifacts: Vec::new(),
            staged_retention_refs: Vec::new(),
            payloads: Vec::new(),
        }
    }

    /// Stages an attempt artifact.
    pub fn stage_attempt_artifact(&mut self, artifact: &RunnerJsonArtifact) -> Result<&mut Self> {
        self.staged_artifacts
            .push(self.artifacts.staged_attempt(artifact)?);
        Ok(self)
    }

    /// Stages a side-effect artifact.
    pub(crate) fn stage_side_effect_artifact(
        &mut self,
        artifact: &RunnerJsonArtifact,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<&mut Self> {
        self.staged_artifacts
            .push(
                self.artifacts
                    .staged_side_effect(artifact, ledger_key, invocation_epoch)?,
            );
        Ok(self)
    }

    /// Stages a side-effect artifact and retains it as runtime evidence.
    pub(crate) fn stage_side_effect_runtime_evidence(
        &mut self,
        artifact: &RunnerJsonArtifact,
        side_effect: &RunnerSideEffectBinding,
    ) -> Result<&mut Self> {
        self.stage_side_effect_artifact(
            artifact,
            side_effect.ledger_key.clone(),
            side_effect.invocation_epoch,
        )?;
        self.retain_runtime_evidence(artifact)?;
        Ok(self)
    }

    /// Appends runtime retention refs for one artifact.
    pub fn retain_runtime_evidence(&mut self, artifact: &RunnerJsonArtifact) -> Result<&mut Self> {
        self.staged_retention_refs
            .push(StagedRetentionRefs::runtime_evidence(vec![
                artifact.retention_ref()?
            ]));
        Ok(self)
    }

    /// Stages and retains request/response evidence for an external capability read.
    pub fn record_external_read_evidence<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let artifact = self.artifacts.external_read_evidence(value)?;
        self.stage_attempt_artifact(&artifact)?;
        self.retain_runtime_evidence(&artifact)?;
        Ok(artifact)
    }

    /// Stages a state-output artifact, retains it as runtime evidence, and emits its cell payload.
    pub fn state_output<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let payloads = RunnerPayloadBuilder::new(self.artifacts.ctx);
        let artifact = self.stage_state_output_artifact(value)?;
        self.payload(payloads.cell_produced(&artifact)?);
        Ok(artifact)
    }

    /// Stages a state-output artifact for a fact value and appends the matching fact record.
    pub fn state_output_and_record_fact<T>(
        &mut self,
        input: FactRecordInput<T>,
        producer: RunnerCapabilityBinding,
    ) -> Result<()>
    where
        T: MfmFactType + MfmValue,
    {
        let payloads = RunnerPayloadBuilder::new(self.artifacts.ctx);
        let state_artifact = self.stage_state_output_artifact(&input.fact)?;
        self.record_fact(input, producer)?;
        self.payload(payloads.cell_produced(&state_artifact)?);
        Ok(())
    }

    /// Stages a state-output artifact and private fact-query replay evidence.
    pub fn state_output_and_record_fact_query_evidence<T>(
        &mut self,
        value: &T,
        evidence: mfm_facts::FactQueryEvidence,
    ) -> Result<()>
    where
        T: MfmValue,
    {
        self.state_output(value)?;
        self.record_fact_query_evidence(evidence)
    }

    /// Stages a typed fact response artifact and appends the matching `FactRecorded` payload.
    pub fn record_fact<T>(
        &mut self,
        input: FactRecordInput<T>,
        producer: RunnerCapabilityBinding,
    ) -> Result<()>
    where
        T: MfmFactType,
    {
        let descriptor = T::descriptor().map_err(runtime_fact_error)?;
        ensure_fact_descriptor_matches_type::<T>(&descriptor)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        ensure_node_allows_fact_descriptor(self.artifacts.ctx.node(), &descriptor_hash)?;

        let subject_json = serde_json::to_value(input.fact.subject())
            .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
        let subject = mfm_facts::typed_fact_subject_evidence(&descriptor, &subject_json)
            .map_err(runtime_fact_error)?;
        let response = self.artifacts.fact_response(input.fact.response())?;
        let response_schema_id = artifact_schema_id(&response)?;
        if descriptor.response_schema_id() != &response_schema_id {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact response schema {} did not match descriptor response schema {}",
                response_schema_id,
                descriptor.response_schema_id()
            )));
        }
        let response_evidence = response.evidence().clone();
        let claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
            visibility: input.visibility,
            fact_kind: descriptor.fact_kind().clone(),
            fact_descriptor_hash: descriptor_hash,
            subject,
            observed_at: input.observed_at,
            request: None,
            response: mfm_facts::FactResponseEvidence::new(
                response_schema_id,
                response_evidence.digest.clone(),
                response_evidence.artifact_id.clone(),
                response_evidence.evidence_hash()?,
            ),
            producer: mfm_facts::FactProducerProvenance::new(
                producer.capability_kind,
                producer.capability_version,
                producer.adapter_kind,
                producer.adapter_version,
            ),
        })
        .map_err(runtime_fact_error)?;

        self.stage_attempt_artifact(&response)?;
        self.payload(RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
            events::FactRecorded {
                spec_hash: self.artifacts.ctx.spec_hash().clone(),
                node_id: self.artifacts.ctx.node().node_id.clone(),
                attempt_id: self.artifacts.ctx.attempt_id().clone(),
                claim,
            },
        )));

        Ok(())
    }

    /// Stages private replay evidence for a live fact query.
    ///
    /// The commit planner emits the existing generic `ArtifactReferenced` event for the staged
    /// artifact. The evidence is retained as runtime evidence and is not represented as a
    /// `FactRecorded` claim.
    pub fn record_fact_query_evidence(
        &mut self,
        evidence: mfm_facts::FactQueryEvidence,
    ) -> Result<()> {
        mfm_facts::validate_fact_query_evidence(&evidence).map_err(runtime_fact_error)?;
        let artifact = self.artifacts.fact_query_evidence(&evidence)?;
        let staged = self.artifacts.staged_attempt(&artifact)?;
        let retention_refs = fact_query_evidence_retention_refs(
            &artifact,
            &evidence,
            self.artifacts.ctx.projections(),
        )?;
        let returned_refs = evidence.receipt().returned_refs().to_vec();
        self.staged_artifacts.push(staged);
        self.staged_retention_refs
            .push(StagedRetentionRefs::fact_query_evidence(
                retention_refs,
                returned_refs,
            ));
        Ok(())
    }

    fn stage_state_output_artifact<T>(&mut self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        let artifact = self.artifacts.state_output(value)?;
        self.stage_attempt_artifact(&artifact)?;
        self.retain_runtime_evidence(&artifact)?;
        Ok(artifact)
    }

    /// Appends a runner-owned event payload.
    pub fn payload(&mut self, payload: RunnerEventPayload) -> &mut Self {
        self.payloads.push(payload);
        self
    }

    /// Finishes the builder into an erased runner output batch.
    pub fn finish(self) -> ErasedRunnerOutput {
        ErasedRunnerOutput::from_parts(
            self.staged_artifacts,
            self.staged_retention_refs,
            self.payloads,
        )
    }
}

impl ErasedRunnerOutput {
    /// Builds an erased runner output containing one state-output value.
    pub fn state_output<T>(ctx: &ErasedRunCtx<'_>, value: &T) -> Result<Self>
    where
        T: MfmValue,
    {
        let mut output = RunnerOutputBuilder::new(ctx);
        output.state_output(value)?;
        Ok(output.finish())
    }
}

pub(crate) fn fact_query_evidence_retention_refs(
    evidence_artifact: &RunnerJsonArtifact,
    evidence: &mfm_facts::FactQueryEvidence,
    projections: &store::ProjectionSnapshot,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = BTreeMap::<(ArtifactId, ContentDigest), events::RetentionRef>::new();
    insert_retention_ref(&mut refs, evidence_artifact.retention_ref()?);

    for fact_ref in evidence.receipt().returned_refs() {
        for retention_ref in fact_query_returned_ref_retention_refs(projections, fact_ref)? {
            insert_retention_ref(&mut refs, retention_ref);
        }
    }

    Ok(refs.into_values().collect())
}

fn insert_retention_ref(
    refs: &mut BTreeMap<(ArtifactId, ContentDigest), events::RetentionRef>,
    retention_ref: events::RetentionRef,
) {
    refs.entry((
        retention_ref.artifact_id.clone(),
        retention_ref.evidence_hash.clone(),
    ))
    .or_insert(retention_ref);
}

fn ensure_artifact_role(artifact: &RunnerJsonArtifact, role: events::ArtifactRole) -> Result<()> {
    if artifact.evidence.artifact_role == role {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not match expected role {}",
            artifact.evidence.artifact_role.as_str(),
            role.as_str()
        )))
    }
}

fn artifact_schema_id(artifact: &RunnerJsonArtifact) -> Result<SchemaId> {
    artifact.evidence.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not carry schema metadata",
            artifact.evidence.artifact_role.as_str()
        ))
    })
}

fn ensure_fact_descriptor_matches_type<T>(descriptor: &mfm_facts::FactDescriptor) -> Result<()>
where
    T: MfmFactType,
{
    let subject_schema_id = <T::Subject as MfmValue>::schema_id().map_err(runtime_value_error)?;
    let response_schema_id = <T::Response as MfmValue>::schema_id().map_err(runtime_value_error)?;

    if descriptor.subject_schema_id() != &subject_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact subject schema {} did not match subject type schema {}",
            descriptor.subject_schema_id(),
            subject_schema_id
        )));
    }
    if descriptor.response_schema_id() != &response_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact response schema {} did not match response type schema {}",
            descriptor.response_schema_id(),
            response_schema_id
        )));
    }

    Ok(())
}

fn ensure_node_allows_fact_descriptor(
    node: &spec::NodeSpec,
    descriptor_hash: &ContentDigest,
) -> Result<()> {
    if node
        .fact_descriptor_allowlist
        .iter()
        .any(|reference| &reference.descriptor_hash == descriptor_hash)
    {
        return Ok(());
    }
    Err(RuntimeError::InvalidRunnerOutput(format!(
        "node {} is not certified to emit fact descriptor {}",
        node.node_id, descriptor_hash
    )))
}

fn canonical_json<T>(value: &T) -> Result<PlainCanonicalJsonBytes>
where
    T: Serialize,
{
    let json =
        serde_json::to_string(value).map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn runtime_value_error(error: mfm_values::ValueError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_fact_error(error: mfm_facts::FactError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}
