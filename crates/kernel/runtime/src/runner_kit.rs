use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    SchemaId,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::Serialize;

use crate::{
    ErasedRunCtx, ErasedRunnerOutput, Result, RunnerEventPayload, RuntimeError, StagedArtifact,
    StagedRetentionRefs,
};

/// Canonical JSON artifact prepared by a typed runner before runtime staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerJsonArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

impl RunnerJsonArtifact {
    /// Returns the canonical JSON bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the artifact evidence bound to the canonical bytes.
    pub fn evidence(&self) -> &store::ArtifactEvidenceRef {
        &self.evidence
    }

    /// Returns a runtime retention ref for this artifact.
    pub fn retention_ref(&self) -> events::RetentionRef {
        events::RetentionRef {
            artifact_id: self.evidence.artifact_id.clone(),
            role: self.evidence.artifact_role,
            content_digest: self.evidence.digest.clone(),
        }
    }

    /// Splits the artifact into canonical bytes and evidence.
    pub fn into_parts(self) -> (Vec<u8>, store::ArtifactEvidenceRef) {
        (self.bytes, self.evidence)
    }
}

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

    /// Builds a side-effect intent artifact.
    pub fn side_effect_intent<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SideEffectIntent)
    }

    /// Builds a schema-less prepared invocation artifact.
    pub fn prepared_invocation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: Serialize,
    {
        self.json_artifact(value, events::ArtifactRole::PreparedInvocation, None, None)
    }

    /// Builds a not-submitted proof artifact.
    pub fn not_submitted_proof<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::NotSubmittedProof)
    }

    /// Builds a submission artifact.
    pub fn submission<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Submission)
    }

    /// Builds a submission-unknown evidence artifact.
    pub fn submission_unknown<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::SubmissionUnknownEvidence)
    }

    /// Builds a receipt artifact.
    pub fn receipt<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Receipt)
    }

    /// Builds a confirmation artifact.
    pub fn confirmation<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
    where
        T: MfmValue,
    {
        self.evidence_artifact(value, events::ArtifactRole::Confirmation)
    }

    /// Builds an ambiguity evidence artifact.
    pub fn ambiguity_evidence<T>(&self, value: &T) -> Result<RunnerJsonArtifact>
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
    pub fn staged_side_effect(
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
        let digest = bytes.content_digest();
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: bytes.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json")?,
            schema_id,
            semantic_type_id,
            producer_node_id: Some(self.ctx.node().node_id.clone()),
            producer_seed_id: None,
            artifact_role: role,
        };
        Ok(RunnerJsonArtifact {
            bytes: bytes.to_vec(),
            evidence,
        })
    }
}

/// Capability and adapter binding metadata for runner payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerCapabilityBinding {
    /// Capability kind used by the runner.
    pub capability_kind: CapabilityKind,
    /// Capability version used by the runner.
    pub capability_version: CapabilityVersion,
    /// Adapter kind used by the runner.
    pub adapter_kind: AdapterKind,
    /// Adapter version used by the runner.
    pub adapter_version: AdapterVersion,
}

/// Shared side-effect ledger coordinates for runner payload builders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSideEffectBinding {
    /// Side-effect ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Invocation epoch.
    pub invocation_epoch: u32,
}

/// Claim metadata for a side-effect invocation owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerClaimBinding {
    /// Claim owner.
    pub claim_owner: events::RunnerInvocationId,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Claim takeover metadata for a side-effect invocation owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerClaimTakeoverBinding {
    /// Previous claim owner.
    pub previous_claim_owner: events::RunnerInvocationId,
    /// New claim owner.
    pub new_claim_owner: events::RunnerInvocationId,
    /// Previous claim generation.
    pub previous_claim_generation: u32,
    /// New claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Prepared invocation metadata for a side-effect runner payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerPreparedInvocationBinding {
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
    /// Optional exclusive resource lane key evidence.
    pub resource_key: Option<events::ResourceKeyEvidence>,
}

/// Builder for runner-owned event payloads.
pub struct RunnerPayloadBuilder<'a, 'ctx> {
    ctx: &'a ErasedRunCtx<'ctx>,
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
            artifact_id: artifact.evidence.artifact_id.clone(),
            content_digest: artifact.evidence.digest.clone(),
            producer_state_kind: Some(self.ctx.node().state_kind.clone()),
            producer_state_version: Some(self.ctx.node().state_version.clone()),
        }))
    }

    /// Builds a `FactRecorded` runner payload from a typed request and fact-response artifact.
    pub fn fact_recorded<Request>(
        &self,
        fact_key: events::FactKey,
        request: &Request,
        response: &RunnerJsonArtifact,
        binding: RunnerCapabilityBinding,
    ) -> Result<RunnerEventPayload>
    where
        Request: MfmValue,
    {
        ensure_artifact_role(response, events::ArtifactRole::FactResponse)?;
        Ok(RunnerEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            capability_kind: binding.capability_kind,
            capability_version: binding.capability_version,
            adapter_kind: binding.adapter_kind,
            adapter_version: binding.adapter_version,
            request_schema_id: Request::schema_id().map_err(runtime_value_error)?,
            request_hash: canonical_json(request)?.content_digest(),
            response_schema_id: artifact_schema_id(response)?,
            response_hash: response.evidence.digest.clone(),
            fact_key,
            artifact_id: response.evidence.artifact_id.clone(),
        }))
    }

    /// Builds a `SideEffectIntentPersisted` runner payload.
    pub fn side_effect_intent_persisted<Idempotency>(
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
            side_effect::IntentPersisted {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                scope_id: self.ctx.node().scope_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                intent_schema_id: artifact_schema_id(intent)?,
                intent_hash: intent.evidence.digest.clone(),
                intent_artifact_id: intent.evidence.artifact_id.clone(),
                idempotency_input_schema_id: Idempotency::schema_id()
                    .map_err(runtime_value_error)?,
                idempotency_input_hash: canonical_json(idempotency)?.content_digest(),
                idempotency_key,
                capability_kind: binding.capability_kind,
                capability_version: binding.capability_version,
                adapter_kind: binding.adapter_kind,
                adapter_version: binding.adapter_version,
            },
        ))
    }

    /// Builds a `SideEffectClaimed` runner payload.
    pub fn side_effect_claimed(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key,
            ledger_purpose: side_effect.ledger_purpose,
            claim_owner: claim.claim_owner,
            invocation_epoch: side_effect.invocation_epoch,
            claim_generation: claim.claim_generation,
            claim_fencing_token: claim.claim_fencing_token,
        })
    }

    /// Builds a `SideEffectClaimTakenOver` runner payload.
    pub fn side_effect_claim_taken_over(
        &self,
        side_effect: RunnerSideEffectBinding,
        takeover: RunnerClaimTakeoverBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key,
            ledger_purpose: side_effect.ledger_purpose,
            previous_claim_owner: takeover.previous_claim_owner,
            new_claim_owner: takeover.new_claim_owner,
            invocation_epoch: side_effect.invocation_epoch,
            previous_claim_generation: takeover.previous_claim_generation,
            claim_generation: takeover.claim_generation,
            claim_fencing_token: takeover.claim_fencing_token,
        })
    }

    /// Builds a `SideEffectInvocationPrepared` runner payload.
    pub fn side_effect_invocation_prepared(
        &self,
        side_effect: RunnerSideEffectBinding,
        prepared: Option<&RunnerJsonArtifact>,
        prepared_binding: RunnerPreparedInvocationBinding,
    ) -> Result<RunnerEventPayload> {
        if let Some(artifact) = prepared {
            ensure_artifact_role(artifact, events::ArtifactRole::PreparedInvocation)?;
        }
        Ok(RunnerEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                claim_generation: prepared_binding.claim_generation,
                claim_fencing_token: prepared_binding.claim_fencing_token,
                prepared_artifact_id: prepared
                    .map(|artifact| artifact.evidence.artifact_id.clone()),
                prepared_hash: prepared.map(|artifact| artifact.evidence.digest.clone()),
                resource_key: prepared_binding.resource_key,
            },
        ))
    }

    /// Builds a `SideEffectInvocationStarted` runner payload.
    pub fn side_effect_invocation_started(
        &self,
        side_effect: RunnerSideEffectBinding,
        claim: RunnerClaimBinding,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key,
            ledger_purpose: side_effect.ledger_purpose,
            invocation_epoch: side_effect.invocation_epoch,
            claim_owner: claim.claim_owner,
            claim_generation: claim.claim_generation,
            claim_fencing_token: claim.claim_fencing_token,
        })
    }

    /// Builds a `SideEffectNotSubmittedProven` runner payload.
    pub fn side_effect_not_submitted_proven(
        &self,
        side_effect: RunnerSideEffectBinding,
        proof: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(proof, events::ArtifactRole::NotSubmittedProof)?;
        Ok(RunnerEventPayload::SideEffectNotSubmittedProven(
            side_effect::NotSubmittedProven {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                proof_schema_id: artifact_schema_id(proof)?,
                proof_hash: proof.evidence.digest.clone(),
                proof_artifact_id: proof.evidence.artifact_id.clone(),
            },
        ))
    }

    /// Builds a `SideEffectSubmissionObserved` runner payload.
    pub fn side_effect_submission_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        submission: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(submission, events::ArtifactRole::Submission)?;
        Ok(RunnerEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                submission_schema_id: artifact_schema_id(submission)?,
                submission_hash: submission.evidence.digest.clone(),
                submission_artifact_id: submission.evidence.artifact_id.clone(),
            },
        ))
    }

    /// Builds a `SideEffectSubmissionUnknown` runner payload.
    pub fn side_effect_submission_unknown(
        &self,
        side_effect: RunnerSideEffectBinding,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::SubmissionUnknownEvidence)?;
        Ok(RunnerEventPayload::SideEffectSubmissionUnknown(
            side_effect::SubmissionUnknown {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                evidence_schema_id: artifact_schema_id(evidence)?,
                evidence_hash: evidence.evidence.digest.clone(),
                evidence_artifact_id: evidence.evidence.artifact_id.clone(),
            },
        ))
    }

    /// Builds a `SideEffectReceiptObserved` runner payload.
    pub fn side_effect_receipt_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        receipt: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(receipt, events::ArtifactRole::Receipt)?;
        Ok(RunnerEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                receipt_schema_id: artifact_schema_id(receipt)?,
                receipt_hash: receipt.evidence.digest.clone(),
                receipt_artifact_id: receipt.evidence.artifact_id.clone(),
                replay_verifier_id,
                resource_touched_set,
            },
        ))
    }

    /// Builds a `SideEffectConfirmationObserved` runner payload.
    pub fn side_effect_confirmation_observed(
        &self,
        side_effect: RunnerSideEffectBinding,
        confirmation: &RunnerJsonArtifact,
        replay_verifier_id: events::ReplayVerifierId,
        resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(confirmation, events::ArtifactRole::Confirmation)?;
        Ok(RunnerEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                confirmation_schema_id: artifact_schema_id(confirmation)?,
                confirmation_hash: confirmation.evidence.digest.clone(),
                confirmation_artifact_id: confirmation.evidence.artifact_id.clone(),
                replay_verifier_id,
                resource_touched_set,
            },
        ))
    }

    /// Builds a `SideEffectAmbiguous` runner payload.
    pub fn side_effect_ambiguous(
        &self,
        side_effect: RunnerSideEffectBinding,
        ambiguity_code: events::AmbiguityCode,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::AmbiguityEvidence)?;
        Ok(RunnerEventPayload::SideEffectAmbiguous(
            side_effect::Ambiguous {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key,
                ledger_purpose: side_effect.ledger_purpose,
                invocation_epoch: side_effect.invocation_epoch,
                ambiguity_code,
                evidence_schema_id: artifact_schema_id(evidence)?,
                evidence_hash: evidence.evidence.digest.clone(),
                evidence_artifact_id: evidence.evidence.artifact_id.clone(),
            },
        ))
    }

    /// Builds a `SideEffectFailed` runner payload.
    pub fn side_effect_failed(
        &self,
        side_effect: RunnerSideEffectBinding,
        failure_phase: side_effect::FailurePhase,
        retryable: bool,
        error: events::MfmErrorInfo,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectFailed(side_effect::Failed {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key,
            ledger_purpose: side_effect.ledger_purpose,
            invocation_epoch: side_effect.invocation_epoch,
            failure_phase,
            retryable,
            error,
        })
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
    pub fn stage_side_effect_artifact(
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

    /// Appends runtime retention refs for one artifact.
    pub fn retain_runtime_evidence(&mut self, artifact: &RunnerJsonArtifact) -> &mut Self {
        self.staged_retention_refs
            .push(StagedRetentionRefs::runtime_evidence(vec![
                artifact.retention_ref()
            ]));
        self
    }

    /// Appends a runner-owned event payload.
    pub fn payload(&mut self, payload: RunnerEventPayload) -> &mut Self {
        self.payloads.push(payload);
        self
    }

    /// Finishes the builder into an erased runner output batch.
    pub fn finish(self) -> ErasedRunnerOutput {
        ErasedRunnerOutput {
            staged_artifacts: self.staged_artifacts,
            staged_retention_refs: self.staged_retention_refs,
            payloads: self.payloads,
        }
    }
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
