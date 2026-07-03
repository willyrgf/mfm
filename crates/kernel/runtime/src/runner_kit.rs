use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, CanonicalValue, DecimalString, PlainCanonicalJsonBytes};
use mfm_capabilities::CapabilitySetDescriptor;
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DescriptorId, DigestAlgorithm, SchemaId,
};
use mfm_program::MfmFactType;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::Serialize;

use crate::{
    artifacts::validate_fact_query_returned_ref_authority, AdapterExecutableBinding,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding,
    ErasedRunnerOutput, ErasedRunnerRegistry, Result, RunnerEventPayload, RunnerFactRecorded,
    RuntimeError, StagedArtifact, StagedRetentionRefs,
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

/// Runner-owned input for recording one typed fact claim.
pub struct FactRecordInput<T: MfmFactType> {
    /// Typed fact value containing the subject and response material.
    pub fact: T,
    /// Visibility selected for the recorded claim.
    pub visibility: mfm_facts::FactVisibility,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
}

impl<T: MfmFactType> FactRecordInput<T> {
    /// Creates fact record input with no source observation timestamp.
    pub fn new(fact: T, visibility: mfm_facts::FactVisibility) -> Self {
        Self {
            fact,
            visibility,
            observed_at: None,
        }
    }

    /// Sets the source observation timestamp.
    pub fn observed_at(mut self, observed_at: impl Into<String>) -> Self {
        self.observed_at = Some(observed_at.into());
        self
    }
}

/// Handle returned after a typed fact has been staged into runner output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFactRecord {
    fact_key: mfm_facts::FactKey,
    response_artifact_id: ArtifactId,
    response_hash: ContentDigest,
}

impl StagedFactRecord {
    /// Returns the descriptor-derived fact key for the staged claim.
    pub const fn fact_key(&self) -> &mfm_facts::FactKey {
        &self.fact_key
    }

    /// Returns the staged response artifact id.
    pub const fn response_artifact_id(&self) -> &ArtifactId {
        &self.response_artifact_id
    }

    /// Returns the staged response content hash.
    pub const fn response_hash(&self) -> &ContentDigest {
        &self.response_hash
    }
}

/// Handle returned after fact query replay evidence has been staged into runner output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFactQueryEvidence {
    artifact_id: ArtifactId,
    evidence_hash: ContentDigest,
}

impl StagedFactQueryEvidence {
    /// Returns the staged query evidence artifact id.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Returns the canonical query evidence artifact-evidence hash.
    pub const fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
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
    /// Certified side-effect pair id.
    pub pair_id: mfm_ids::SideEffectPairId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
}

impl RunnerSideEffectBinding {
    pub(crate) fn pair_role(&self, role: events::SideEffectPairRole) -> events::SideEffectPairRole {
        role
    }
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
    /// Optional exclusive resource lane key evidence echoed from the held lane.
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
            skip_reason,
        })
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
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
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
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
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
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
            previous_claim_owner: takeover.previous_claim_owner,
            new_claim_owner: takeover.new_claim_owner,
            invocation_epoch: side_effect.invocation_epoch,
            previous_claim_generation: takeover.previous_claim_generation,
            claim_generation: takeover.claim_generation,
            claim_fencing_token: takeover.claim_fencing_token,
        })
    }

    /// Builds a `ResourceLaneClaimIntent` runner payload for pre-invocation lane authority.
    pub fn resource_lane_claim_intent(
        &self,
        side_effect: RunnerSideEffectBinding,
        resource_key: events::ResourceKeyEvidence,
        requirement_digest: ContentDigest,
        resolved_by_capability_impl: events::RunnerFactoryId,
    ) -> RunnerEventPayload {
        RunnerEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
            invocation_epoch: side_effect.invocation_epoch,
            resource_key,
            requirement_digest,
            resolved_by_capability_impl,
        })
    }

    /// Builds a `ResourceLaneReleaseIntent` runner payload for terminal lane release.
    pub fn resource_lane_release_intent(
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
            pair_role: side_effect.pair_role(events::SideEffectPairRole::Verify),
            invocation_epoch: side_effect.invocation_epoch,
            claim_id,
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason,
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
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
                invocation_epoch: side_effect.invocation_epoch,
                claim_generation: prepared_binding.claim_generation,
                claim_fencing_token: prepared_binding.claim_fencing_token,
                resource_key: prepared_binding.resource_key,
                prepared_artifact_id: prepared
                    .map(|artifact| artifact.evidence.artifact_id.clone()),
                prepared_hash: prepared.map(|artifact| artifact.evidence.digest.clone()),
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
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
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
        self.side_effect_not_submitted_proven_with_role(
            side_effect,
            events::SideEffectPairRole::Submit,
            proof,
        )
    }

    /// Builds a `SideEffectNotSubmittedProven` runner payload with an explicit pair role.
    pub fn side_effect_not_submitted_proven_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        proof: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(proof, events::ArtifactRole::NotSubmittedProof)?;
        Ok(RunnerEventPayload::SideEffectNotSubmittedProven(
            side_effect::NotSubmittedProven {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(pair_role),
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
        self.side_effect_submission_observed_with_role(
            side_effect,
            events::SideEffectPairRole::Submit,
            submission,
        )
    }

    /// Builds a `SideEffectSubmissionObserved` runner payload with an explicit pair role.
    pub fn side_effect_submission_observed_with_role(
        &self,
        side_effect: RunnerSideEffectBinding,
        pair_role: events::SideEffectPairRole,
        submission: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(submission, events::ArtifactRole::Submission)?;
        Ok(RunnerEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(pair_role),
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
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(events::SideEffectPairRole::Submit),
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
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(events::SideEffectPairRole::Verify),
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
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(events::SideEffectPairRole::Verify),
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
        pair_role: events::SideEffectPairRole,
        ambiguity_code: events::AmbiguityCode,
        evidence: &RunnerJsonArtifact,
    ) -> Result<RunnerEventPayload> {
        ensure_artifact_role(evidence, events::ArtifactRole::AmbiguityEvidence)?;
        Ok(RunnerEventPayload::SideEffectAmbiguous(
            side_effect::Ambiguous {
                spec_hash: self.ctx.spec_hash().clone(),
                node_id: self.ctx.node().node_id.clone(),
                attempt_id: self.ctx.attempt_id().clone(),
                ledger_key: side_effect.ledger_key.clone(),
                ledger_purpose: side_effect.ledger_purpose.clone(),
                pair_id: side_effect.pair_id.clone(),
                pair_role: side_effect.pair_role(pair_role),
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
        pair_role: events::SideEffectPairRole,
        failure_phase: side_effect::FailurePhase,
        retryable: bool,
        error: events::MfmErrorInfo,
    ) -> RunnerEventPayload {
        RunnerEventPayload::SideEffectFailed(side_effect::Failed {
            spec_hash: self.ctx.spec_hash().clone(),
            node_id: self.ctx.node().node_id.clone(),
            attempt_id: self.ctx.attempt_id().clone(),
            ledger_key: side_effect.ledger_key.clone(),
            ledger_purpose: side_effect.ledger_purpose.clone(),
            pair_id: side_effect.pair_id.clone(),
            pair_role: side_effect.pair_role(pair_role),
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

    /// Stages a typed fact response artifact and appends the matching `FactRecorded` payload.
    pub fn record_fact<T>(
        &mut self,
        input: FactRecordInput<T>,
        producer: RunnerCapabilityBinding,
    ) -> Result<StagedFactRecord>
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
        let subject_value =
            typed_fact_subject_value(&descriptor, &subject_json).map_err(runtime_fact_error)?;
        let subject_material = mfm_facts::extract_subject_material(&descriptor, &subject_value)
            .map_err(runtime_fact_error)?;
        let subject_material_hash =
            mfm_facts::subject_material_hash(&subject_material).map_err(runtime_fact_error)?;
        let subject_namespace =
            mfm_facts::fact_subject_namespace(&descriptor).map_err(runtime_fact_error)?;
        let subject_namespace_hash = mfm_facts::fact_subject_namespace_hash(&subject_namespace)
            .map_err(runtime_fact_error)?;
        let fact_key = mfm_facts::derive_fact_key(
            subject_namespace_hash.clone(),
            subject_material_hash.clone(),
        )
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
            subject: mfm_facts::FactSubjectEvidence::from_material(
                subject_namespace_hash,
                &subject_material,
            )
            .map_err(runtime_fact_error)?,
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

        self.staged_artifacts
            .push(self.artifacts.staged_attempt(&response)?);
        self.payloads
            .push(RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                events::FactRecorded {
                    spec_hash: self.artifacts.ctx.spec_hash().clone(),
                    node_id: self.artifacts.ctx.node().node_id.clone(),
                    attempt_id: self.artifacts.ctx.attempt_id().clone(),
                    claim,
                },
            )));

        Ok(StagedFactRecord {
            fact_key,
            response_artifact_id: response_evidence.artifact_id,
            response_hash: response_evidence.digest,
        })
    }

    /// Stages private replay evidence for a live fact query.
    ///
    /// The commit planner emits the existing generic `ArtifactReferenced` event for the staged
    /// artifact. The evidence is retained as runtime evidence and is not represented as a
    /// `FactRecorded` claim.
    pub fn record_fact_query_evidence(
        &mut self,
        evidence: mfm_facts::FactQueryEvidence,
        trust_root: &store::FactQueryReceiptTrustRoot,
    ) -> Result<StagedFactQueryEvidence> {
        store::validate_fact_query_evidence_recording(&evidence, trust_root)
            .map_err(RuntimeError::from)?;
        let artifact = self.artifacts.fact_query_evidence(&evidence)?;
        let staged = self.artifacts.staged_attempt(&artifact)?;
        let staged_evidence = staged.evidence().clone();
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
        let evidence_hash = staged_evidence.evidence_hash()?;
        Ok(StagedFactQueryEvidence {
            artifact_id: staged_evidence.artifact_id,
            evidence_hash,
        })
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

fn fact_query_evidence_retention_refs(
    evidence_artifact: &RunnerJsonArtifact,
    evidence: &mfm_facts::FactQueryEvidence,
    projections: &store::ProjectionSnapshot,
) -> Result<Vec<events::RetentionRef>> {
    let mut refs = BTreeMap::<(ArtifactId, events::ArtifactRole), events::RetentionRef>::new();
    insert_retention_ref(&mut refs, evidence_artifact.retention_ref());

    for fact_ref in evidence.receipt().returned_refs() {
        validate_fact_query_returned_ref_authority(projections, fact_ref)?;
        let descriptor = projections
            .fact_descriptor(fact_ref.fact_descriptor_hash())
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(
                    "fact query evidence returned ref missing descriptor authority".to_owned(),
                )
            })?;
        insert_retention_ref(
            &mut refs,
            events::RetentionRef {
                artifact_id: descriptor.descriptor_artifact_id.clone(),
                role: events::ArtifactRole::FactDescriptor,
                content_digest: descriptor.descriptor_hash.clone(),
            },
        );
        insert_retention_ref(
            &mut refs,
            events::RetentionRef {
                artifact_id: fact_ref.artifact_id().clone(),
                role: events::ArtifactRole::FactResponse,
                content_digest: fact_ref.response_hash().clone(),
            },
        );
    }

    Ok(refs.into_values().collect())
}

fn insert_retention_ref(
    refs: &mut BTreeMap<(ArtifactId, events::ArtifactRole), events::RetentionRef>,
    retention_ref: events::RetentionRef,
) {
    refs.entry((retention_ref.artifact_id.clone(), retention_ref.role))
        .or_insert(retention_ref);
}

/// Builder for registering runner bindings while keeping executable identity explicit.
pub struct RunnerRegistrationBuilder<'a> {
    registry: &'a mut ErasedRunnerRegistry,
    implementation_id: CapabilityImplementationId,
}

impl<'a> RunnerRegistrationBuilder<'a> {
    /// Creates a registration builder for one concrete capability implementation id.
    pub fn new(
        registry: &'a mut ErasedRunnerRegistry,
        implementation_id: CapabilityImplementationId,
    ) -> Self {
        Self {
            registry,
            implementation_id,
        }
    }

    /// Registers the configured implementation id for a certified capability set.
    pub fn register_capability_set(
        &mut self,
        capabilities: &CapabilitySetDescriptor,
    ) -> Result<&mut Self> {
        self.registry
            .register_capability_set(capabilities, self.implementation_id.clone())?;
        Ok(self)
    }

    /// Registers one descriptor runner using caller-supplied factory and executable identity.
    pub fn register_runner(
        &mut self,
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        let binding = ErasedRunnerBinding::new(descriptor_id, factory_id, executable, runner)?;
        self.registry.register(binding)?;
        Ok(self)
    }

    /// Registers capabilities and a runner for one certified state descriptor.
    pub fn register_descriptor(
        &mut self,
        descriptor_id: DescriptorId,
        capabilities: &CapabilitySetDescriptor,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.register_capability_set(capabilities)?;
        self.register_runner(descriptor_id, factory_id, executable, runner)
    }

    /// Registers the adapter-owned runner used by side-effect verify framework nodes
    /// for one certified side-effect submit descriptor.
    pub fn register_side_effect_verify_runner(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.registry.register_side_effect_verify_runner(
            submit_descriptor_id,
            factory_id,
            executable,
            runner,
        )?;
        Ok(self)
    }

    /// Registers executable evidence for one certified adapter binding.
    pub fn register_adapter_executable(
        &mut self,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        executable: events::ExecutableIdentity,
    ) -> Result<&mut Self> {
        self.registry
            .register_adapter_executable(AdapterExecutableBinding::new(
                adapter_kind,
                adapter_version,
                executable,
            ))?;
        Ok(self)
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

fn typed_fact_subject_value(
    descriptor: &mfm_facts::FactDescriptor,
    subject: &serde_json::Value,
) -> std::result::Result<CanonicalValue, mfm_facts::FactDescriptorError> {
    let mut typed_paths = std::collections::BTreeMap::new();
    for field in descriptor.fields() {
        let mfm_facts::FactFieldAccessor::SubjectPath(path) = field.accessor() else {
            continue;
        };
        if let Some(previous) = typed_paths.insert(path.as_str().to_owned(), field.value_type()) {
            if previous != field.value_type() {
                return Err(mfm_facts::FactDescriptorError::descriptor(format!(
                    "subject path {} is declared with incompatible value types",
                    path
                )));
            }
        }
    }
    json_to_fact_canonical_value(subject, "", &typed_paths)
}

fn json_to_fact_canonical_value(
    value: &serde_json::Value,
    path: &str,
    typed_paths: &std::collections::BTreeMap<String, mfm_facts::FactFieldValueType>,
) -> std::result::Result<CanonicalValue, mfm_facts::FactDescriptorError> {
    if let Some(value_type) = typed_paths.get(path) {
        return json_to_typed_fact_scalar(value, *value_type, path);
    }

    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(mfm_facts::FactDescriptorError::descriptor(format!(
                    "fact subject path {path} contains unsupported floating-point number"
                )))
            }
        }
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let child_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                json_to_fact_canonical_value(value, &child_path, typed_paths)
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    Ok((
                        key.clone(),
                        json_to_fact_canonical_value(value, &child_path, typed_paths)?,
                    ))
                })
                .collect::<std::result::Result<Vec<_>, mfm_facts::FactDescriptorError>>()?;
            CanonicalValue::object(values)
                .map_err(|error| mfm_facts::FactDescriptorError::descriptor(error.to_string()))
        }
    }
}

fn json_to_typed_fact_scalar(
    value: &serde_json::Value,
    value_type: mfm_facts::FactFieldValueType,
    path: &str,
) -> std::result::Result<CanonicalValue, mfm_facts::FactDescriptorError> {
    match value_type {
        mfm_facts::FactFieldValueType::String => {
            string_json(value, path).map(|value| CanonicalValue::String(value.to_owned()))
        }
        mfm_facts::FactFieldValueType::Boolean => value
            .as_bool()
            .map(CanonicalValue::Bool)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        mfm_facts::FactFieldValueType::SignedInteger => value
            .as_i64()
            .map(CanonicalValue::Signed)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        mfm_facts::FactFieldValueType::UnsignedInteger => value
            .as_u64()
            .map(CanonicalValue::Unsigned)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        mfm_facts::FactFieldValueType::Timestamp => {
            string_json(value, path).map(|value| CanonicalValue::String(value.to_owned()))
        }
        mfm_facts::FactFieldValueType::DecimalString => {
            let decimal = DecimalString::new_variable(string_json(value, path)?)
                .map_err(|error| mfm_facts::FactDescriptorError::descriptor(error.to_string()))?;
            Ok(CanonicalValue::Decimal(decimal))
        }
        mfm_facts::FactFieldValueType::Digest => {
            string_json(value, path).map(|value| CanonicalValue::String(value.to_owned()))
        }
    }
}

fn string_json<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> std::result::Result<&'a str, mfm_facts::FactDescriptorError> {
    value.as_str().ok_or_else(|| {
        mfm_facts::FactDescriptorError::descriptor(format!(
            "fact subject path {path} is not a string"
        ))
    })
}

fn fact_scalar_type_error(
    path: &str,
    value_type: mfm_facts::FactFieldValueType,
) -> mfm_facts::FactDescriptorError {
    mfm_facts::FactDescriptorError::descriptor(format!(
        "fact subject path {path} does not match {value_type:?}"
    ))
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

fn runtime_fact_error(error: mfm_facts::FactDescriptorError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn run_id(byte: u8) -> mfm_ids::RunId {
        mfm_ids::RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn event_id(byte: u8) -> mfm_ids::EventId {
        mfm_ids::EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([byte; 32]),
        )
    }

    fn schema_id() -> SchemaId {
        SchemaId::new(
            "mfm.test.response",
            "mfm.test.v1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([1; 32]),
        )
        .expect("schema")
    }

    fn capability_kind() -> CapabilityKind {
        CapabilityKind::new(
            "mfm.test.capability",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([2; 32]),
        )
        .expect("capability")
    }

    fn adapter_kind() -> AdapterKind {
        AdapterKind::new(
            "mfm.test.adapter",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([3; 32]),
        )
        .expect("adapter")
    }

    fn fact_subject_evidence(height: u64) -> mfm_facts::FactSubjectEvidence {
        let material =
            mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactSubjectValueV1::new(
                mfm_facts::FactFieldId::new("subject.height").expect("field"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactCanonicalScalar::UnsignedInteger(height),
            )
            .expect("subject value")])
            .expect("subject material");
        mfm_facts::FactSubjectEvidence::from_material(digest(0x13), &material)
            .expect("subject evidence")
    }

    fn fact_producer_node_id() -> mfm_ids::NodeId {
        mfm_ids::NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0x19; 32]),
        )
    }

    fn fact_response_artifact_evidence(
        producer_node_id: &mfm_ids::NodeId,
    ) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id: artifact_id(0x17),
            digest: digest(0x16),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(schema_id()),
            semantic_type_id: None,
            producer_node_id: Some(producer_node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactResponse,
        }
    }

    fn fact_ref() -> mfm_facts::InternalFactRef {
        let subject = fact_subject_evidence(17);
        let producer_node_id = fact_producer_node_id();
        let response_evidence = fact_response_artifact_evidence(&producer_node_id);
        let artifact_evidence_hash = response_evidence
            .evidence_hash()
            .expect("response artifact evidence hash");
        mfm_facts::InternalFactRef::new(mfm_facts::InternalFactRefParts {
            fact_claim_id: mfm_facts::FactClaimId::new(run_id(0x10), 1, 0).expect("claim id"),
            source_event_id: event_id(0x11),
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            producer_node_id,
            observed_at: None,
            visibility: mfm_facts::FactVisibility::indexed_default(
                mfm_facts::FactAudience::Platform,
            ),
            fact_kind: mfm_facts::FactKind::new("chain.head").expect("kind"),
            fact_descriptor_hash: digest(0x12),
            fact_subject_namespace_hash: subject.fact_subject_namespace_hash().clone(),
            fact_key: subject.fact_key().clone(),
            subject_material_hash: subject.subject_material_hash().clone(),
            request_schema_id: None,
            request_hash: None,
            response_schema_id: response_evidence
                .schema_id
                .clone()
                .expect("response schema id"),
            response_hash: response_evidence.digest.clone(),
            artifact_id: response_evidence.artifact_id,
            artifact_evidence_hash,
            capability_kind: capability_kind(),
            capability_version: CapabilityVersion::new("mfm.test.capability.v1")
                .expect("capability version"),
            adapter_kind: adapter_kind(),
            adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        })
        .expect("fact ref")
    }

    fn fact_claim_for_ref(
        fact_ref: &mfm_facts::InternalFactRef,
        subject_height: u64,
    ) -> mfm_facts::FactClaim {
        mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
            visibility: fact_ref.visibility().clone(),
            fact_kind: fact_ref.fact_kind().clone(),
            fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            subject: fact_subject_evidence(subject_height),
            observed_at: fact_ref.observed_at().map(str::to_owned),
            request: None,
            response: mfm_facts::FactResponseEvidence::new(
                fact_ref.response_schema_id().clone(),
                fact_ref.response_hash().clone(),
                fact_ref.artifact_id().clone(),
                fact_ref.artifact_evidence_hash().clone(),
            ),
            producer: mfm_facts::FactProducerProvenance::new(
                fact_ref.capability_kind().clone(),
                fact_ref.capability_version().clone(),
                fact_ref.adapter_kind().clone(),
                fact_ref.adapter_version().clone(),
            ),
        })
        .expect("fact claim")
    }

    fn fact_authority_projections(
        fact_ref: &mfm_facts::InternalFactRef,
    ) -> store::ProjectionSnapshot {
        let descriptor_artifact_evidence = store::ArtifactEvidenceRef {
            artifact_id: artifact_id(0x40),
            digest: fact_ref.fact_descriptor_hash().clone(),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(schema_id()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactDescriptor,
        };
        let response_artifact_evidence =
            fact_response_artifact_evidence(fact_ref.producer_node_id());
        let descriptor_projection = store::FactDescriptorProjection {
            descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            descriptor_artifact_id: descriptor_artifact_evidence.artifact_id.clone(),
            descriptor_artifact_evidence,
            fact_kind: fact_ref.fact_kind().clone(),
            descriptor_schema_id: schema_id(),
            subject_schema_id: schema_id(),
            response_schema_id: fact_ref.response_schema_id().clone(),
            fact_subject_namespace_hash: fact_ref.fact_subject_namespace_hash().clone(),
            source_event_id: event_id(0x41),
        };
        let record_projection = store::FactRecordProjection {
            fact_claim_id: fact_ref.fact_claim_id().clone(),
            source_event_id: fact_ref.source_event_id().clone(),
            source_run_id: fact_ref.fact_claim_id().source_run_id().clone(),
            source_seq: fact_ref.fact_claim_id().source_seq(),
            source_ordinal: fact_ref.fact_claim_id().source_ordinal(),
            node_id: fact_ref.producer_node_id().clone(),
            attempt_id: mfm_ids::AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                mfm_ids::DigestBytes::from_array([0x42; 32]),
            ),
            response_artifact_evidence: Some(response_artifact_evidence),
            claim: fact_claim_for_ref(fact_ref, 17),
        };
        let index_projection = store::FactIndexProjection {
            fact_claim_id: fact_ref.fact_claim_id().clone(),
            source_run_id: fact_ref.fact_claim_id().source_run_id().clone(),
            source_seq: fact_ref.fact_claim_id().source_seq(),
            source_ordinal: fact_ref.fact_claim_id().source_ordinal(),
            source_event_id: fact_ref.source_event_id().clone(),
            producer_node_id: fact_ref.producer_node_id().clone(),
            commit_id: store::CommitKey::new("fact-query-authority").expect("commit key"),
            store_commit_order: 1,
            recorded_at: fact_ref.recorded_at().to_owned(),
            observed_at: fact_ref.observed_at().map(str::to_owned),
            audience: mfm_facts::FactAudience::Platform,
            visibility_scope: mfm_facts::FactVisibilityScope::Default,
            fact_kind: fact_ref.fact_kind().clone(),
            fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            fact_subject_namespace_hash: fact_ref.fact_subject_namespace_hash().clone(),
            fact_key: fact_ref.fact_key().clone(),
            subject_material_hash: fact_ref.subject_material_hash().clone(),
            request_schema_id: None,
            request_hash: None,
            response_schema_id: fact_ref.response_schema_id().clone(),
            response_hash: fact_ref.response_hash().clone(),
            artifact_id: fact_ref.artifact_id().clone(),
            artifact_evidence_hash: fact_ref.artifact_evidence_hash().clone(),
            capability_kind: fact_ref.capability_kind().clone(),
            capability_version: fact_ref.capability_version().clone(),
            adapter_kind: fact_ref.adapter_kind().clone(),
            adapter_version: fact_ref.adapter_version().clone(),
        };
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                descriptor_projection.descriptor_hash.clone(),
                descriptor_projection,
            )]),
            fact_records: BTreeMap::from([(
                record_projection.fact_claim_id.clone(),
                record_projection,
            )]),
            fact_index_entries: BTreeMap::from([(
                index_projection.fact_claim_id.clone(),
                index_projection,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("projection snapshot")
    }

    fn fact_authority_projections_without(
        fact_ref: &mfm_facts::InternalFactRef,
        descriptor: bool,
        record: bool,
        index: bool,
    ) -> store::ProjectionSnapshot {
        let full = fact_authority_projections(fact_ref);
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: if descriptor {
                full.fact_descriptors()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            } else {
                BTreeMap::new()
            },
            fact_records: if record {
                full.fact_records()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            } else {
                BTreeMap::new()
            },
            fact_index_entries: if index {
                full.fact_index_entries()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            } else {
                BTreeMap::new()
            },
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("filtered fact authority projection")
    }

    fn fact_authority_projections_with_tampered_subject(
        fact_ref: &mfm_facts::InternalFactRef,
    ) -> store::ProjectionSnapshot {
        let full = fact_authority_projections(fact_ref);
        let mut fact_records = full
            .fact_records()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        fact_records
            .get_mut(fact_ref.fact_claim_id())
            .expect("fact record")
            .claim = fact_claim_for_ref(fact_ref, 18);
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: full
                .fact_descriptors()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            fact_records,
            fact_index_entries: full
                .fact_index_entries()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("tampered fact authority projection")
    }

    fn query_evidence_artifact() -> RunnerJsonArtifact {
        RunnerJsonArtifact {
            bytes: b"{}".to_vec(),
            evidence: store::ArtifactEvidenceRef {
                artifact_id: artifact_id(0x30),
                digest: digest(0x31),
                byte_len: 2,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::FactQueryEvidence,
            },
        }
    }

    fn query_evidence(fact_ref: mfm_facts::InternalFactRef) -> mfm_facts::FactQueryEvidence {
        let query_scope = mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        );
        let store_scope = mfm_facts::StoreScopeRef::new("default").expect("store scope");
        let ordering = mfm_facts::FactOrdering::new(
            mfm_facts::FactOrderingName::new("metadata.store_order.asc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("metadata.store_order").expect("field"),
                mfm_facts::SortDirection::Ascending,
                mfm_facts::NullOrdering::Last,
                true,
            )],
        )
        .expect("ordering");
        let plan = mfm_facts::CanonicalFactQueryPlan::new(
            store_scope.clone(),
            query_scope.clone(),
            mfm_facts::FactQueryCompilerVersion::new(mfm_facts::FACT_QUERY_COMPILER_VERSION)
                .expect("compiler"),
            mfm_facts::FactCanonicalizerVersion::new(mfm_facts::FACT_QUERY_CANONICALIZER_VERSION)
                .expect("canonicalizer"),
            digest(0x12),
            mfm_facts::ScopeDecisionEvidence::new(digest(0x19)),
            mfm_canonical::CanonicalJsonBytes::from_value(
                &CanonicalValue::object([(
                    "kind",
                    CanonicalValue::String("chain.head".to_owned()),
                )])
                .expect("query"),
            ),
            ordering,
            Some(1),
        )
        .expect("plan");
        let frontier = mfm_facts::StoreReadFrontier::new(
            store_scope,
            query_scope,
            mfm_facts::DescriptorCatalogWatermark::new(1),
            mfm_facts::FactProjectionGeneration::new(1),
            1,
            mfm_facts::StoreCommitWatermark::new(1),
        );
        let receipt = mfm_facts::FactQueryReceipt::new(
            frontier,
            mfm_facts::StoreReadFrontierType::Snapshot,
            vec![fact_ref],
            None,
            digest(0x20),
            mfm_facts::QueryResultCardinality::Exact(1),
            digest(0x21),
            mfm_facts::StoreReceiptAuthentication::new(
                mfm_facts::StoreIdentity::new("store.default").expect("store"),
                mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
                Some(mfm_facts::StoreKeyId::new("key.default").expect("key")),
                vec![0; 64],
            )
            .expect("auth"),
        );
        mfm_facts::FactQueryEvidence::new(
            plan,
            receipt,
            mfm_facts::FactSelectionEvidence::new(digest(0x22), Vec::new(), None)
                .expect("selection"),
        )
    }

    #[test]
    fn fact_query_evidence_retention_refs_include_returned_fact_authority_artifacts() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections(&fact_ref);
        let evidence = query_evidence(fact_ref.clone());

        let refs = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect("fact query retention refs");

        assert!(refs.contains(&events::RetentionRef {
            artifact_id: evidence_artifact.evidence.artifact_id.clone(),
            role: events::ArtifactRole::FactQueryEvidence,
            content_digest: evidence_artifact.evidence.digest.clone(),
        }));
        assert!(refs.contains(&events::RetentionRef {
            artifact_id: artifact_id(0x40),
            role: events::ArtifactRole::FactDescriptor,
            content_digest: fact_ref.fact_descriptor_hash().clone(),
        }));
        assert!(refs.contains(&events::RetentionRef {
            artifact_id: fact_ref.artifact_id().clone(),
            role: events::ArtifactRole::FactResponse,
            content_digest: fact_ref.response_hash().clone(),
        }));
    }

    #[test]
    fn fact_query_evidence_retention_refs_reject_missing_descriptor_authority() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections_without(&fact_ref, false, true, true);
        let evidence = query_evidence(fact_ref);

        let error = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect_err("missing descriptor authority rejects");

        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("missing descriptor authority")
        ));
    }

    #[test]
    fn fact_query_evidence_retention_refs_reject_missing_source_fact_authority() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections_without(&fact_ref, true, false, true);
        let evidence = query_evidence(fact_ref);

        let error = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect_err("missing source fact authority rejects");

        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("missing source fact authority")
        ));
    }

    #[test]
    fn fact_query_evidence_retention_refs_reject_missing_indexed_fact_authority() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections_without(&fact_ref, true, true, false);
        let evidence = query_evidence(fact_ref);

        let error = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect_err("missing indexed fact authority rejects");

        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("missing indexed fact authority")
        ));
    }

    #[test]
    fn fact_query_evidence_retention_refs_reject_tampered_subject_material_authority() {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = fact_authority_projections_with_tampered_subject(&fact_ref);
        let evidence = query_evidence(fact_ref);

        let error = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
            .expect_err("tampered source subject authority rejects");

        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("source fact authority does not match")
        ));
    }
}
