use super::*;

/// Retained source fact event authority referenced by pinned fact-query evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedSourceFactReplayEvent {
    fact_claim_id: mfm_facts::FactClaimId,
    envelope: KernelEventEnvelope,
}

impl RetainedSourceFactReplayEvent {
    /// Creates retained source fact event authority after validating the event coordinate.
    pub fn new(
        fact_claim_id: mfm_facts::FactClaimId,
        envelope: KernelEventEnvelope,
    ) -> Result<Self> {
        let expected = mfm_facts::derive_fact_claim_id(
            envelope.run_id().clone(),
            envelope.seq().as_u64(),
            envelope.ordinal().as_u32(),
        )
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
        if expected != fact_claim_id {
            return Err(ReplayError::new(
                ReplayErrorKind::FactMismatch,
                "retained source fact event coordinate does not match fact claim id",
            ));
        }
        if !matches!(envelope.payload(), KernelEventPayload::FactRecorded(_)) {
            return Err(ReplayError::new(
                ReplayErrorKind::FactMismatch,
                "retained source fact event payload is not FactRecorded",
            ));
        }
        Ok(Self {
            fact_claim_id,
            envelope,
        })
    }

    /// Returns the retained source fact claim id.
    pub fn fact_claim_id(&self) -> &mfm_facts::FactClaimId {
        &self.fact_claim_id
    }

    /// Returns the retained source fact event envelope.
    pub fn envelope(&self) -> &KernelEventEnvelope {
        &self.envelope
    }
}

/// Request for replaying a previously recorded read fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactReplayRequest {
    /// Expected producing node id.
    pub node_id: NodeId,
    /// Expected producing attempt id.
    pub attempt_id: AttemptId,
    /// Store-derived claim id for the recorded fact.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Capability kind expected by the replaying state.
    pub capability_kind: CapabilityKind,
    /// Capability version expected by the replaying state.
    pub capability_version: CapabilityVersion,
    /// Adapter kind expected by the replaying state.
    pub adapter_kind: AdapterKind,
    /// Adapter version expected by the replaying state.
    pub adapter_version: AdapterVersion,
    /// Request schema id expected by replay.
    pub request_schema_id: SchemaId,
    /// Canonical request hash expected by replay.
    pub request_hash: ContentDigest,
    /// Response schema id expected by replay.
    pub response_schema_id: SchemaId,
}

/// Replay evidence returned for a recorded read fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFactReplay {
    /// Store-derived claim id for the recorded fact.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Recorded fact event payload.
    pub fact: events::FactRecorded,
    /// Retained artifact evidence for the fact response.
    pub artifact: StoredArtifactEvidenceRef,
}

/// Request for replaying recorded side-effect evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectEvidenceReplayRequest {
    /// Stable side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Invocation epoch to replay.
    pub invocation_epoch: u32,
    /// Certified submit-node context and output resource authority for this side effect.
    pub certified_context: CertifiedSideEffectContext,
    /// Evidence schema id expected by replay.
    pub evidence_schema_id: SchemaId,
    /// Canonical evidence hash expected by replay.
    pub evidence_hash: ContentDigest,
    /// Replay verifier required for receipt and confirmation evidence.
    pub replay_verifier_id: Option<events::ReplayVerifierId>,
}

/// Certified context authority for a side-effect submit node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSideEffectContext {
    /// Certified context required by the submit node.
    pub node_context: spec::NodeContextSpec,
    /// Certified context/resource/stage constraint for the submit node output cell.
    pub output_context: spec::CellContextSpec,
}

impl CertifiedSideEffectContext {
    /// Returns no-context side-effect authority for tests and no-context domains.
    pub const fn no_context() -> Self {
        Self {
            node_context: spec::NodeContextSpec::NoContext,
            output_context: spec::CellContextSpec::NoContext,
        }
    }
}

/// Broker-indexed side-effect replay frame for one intent and invocation epoch.
///
/// The frame borrows recorded, replay-authorized event payloads from [`ReplayBroker`]. Domain
/// verifiers still own cardinality, missing-evidence policy, and evidence interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectReplayFrame<'a> {
    /// Intent persisted event payload.
    pub intent: &'a side_effect::IntentPersisted,
    /// Certified submit-node context and output resource authority.
    pub certified_context: CertifiedSideEffectContext,
    /// Invocation prepared event payload, when present.
    pub prepared: Option<&'a side_effect::InvocationPrepared>,
    /// Submission observed event payload, when present.
    pub submission: Option<&'a side_effect::SubmissionObserved>,
    /// Submission-unknown event payload, when present.
    pub submission_unknown: Option<&'a side_effect::SubmissionUnknown>,
    /// Not-submitted proof payload, when present.
    pub not_submitted: Option<&'a side_effect::NotSubmittedProven>,
    /// Receipt observed event payload, when present.
    pub receipt: Option<&'a side_effect::ReceiptObserved>,
    /// Confirmation observed event payload, when present.
    pub confirmation: Option<&'a side_effect::ConfirmationObserved>,
    /// Ambiguity event payload, when present.
    pub ambiguity: Option<&'a side_effect::Ambiguous>,
}

impl SideEffectReplayFrame<'_> {
    /// Builds the replay request for this frame's prepared invocation, when present.
    pub fn prepared_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let prepared = self.prepared?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            prepared.prepared_schema_id.clone(),
            prepared.prepared_hash.clone(),
            None,
        ))
    }

    /// Builds the replay request for this frame's submission evidence, when present.
    pub fn submission_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let submission = self.submission?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            submission.submission_schema_id.clone(),
            submission.submission_hash.clone(),
            None,
        ))
    }

    /// Builds the replay request for this frame's submission-unknown evidence, when present.
    pub fn submission_unknown_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let unknown = self.submission_unknown?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            unknown.evidence_schema_id.clone(),
            unknown.evidence_hash.clone(),
            None,
        ))
    }

    /// Builds the replay request for this frame's receipt evidence, when present.
    pub fn receipt_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let receipt = self.receipt?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            receipt.receipt_schema_id.clone(),
            receipt.receipt_hash.clone(),
            Some(receipt.replay_verifier_id.clone()),
        ))
    }

    /// Builds the replay request for this frame's not-submitted proof, when present.
    pub fn not_submitted_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let proof = self.not_submitted?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            proof.proof_schema_id.clone(),
            proof.proof_hash.clone(),
            None,
        ))
    }

    /// Builds the replay request for this frame's confirmation evidence, when present.
    pub fn confirmation_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let confirmation = self.confirmation?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            confirmation.confirmation_schema_id.clone(),
            confirmation.confirmation_hash.clone(),
            Some(confirmation.replay_verifier_id.clone()),
        ))
    }

    /// Builds the replay request for this frame's ambiguity evidence, when present.
    pub fn ambiguity_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
        let ambiguity = self.ambiguity?;
        Some(side_effect_evidence_replay_request(
            self.intent,
            self.certified_context.clone(),
            ambiguity.evidence_schema_id.clone(),
            ambiguity.evidence_hash.clone(),
            None,
        ))
    }
}

fn side_effect_evidence_replay_request(
    intent: &side_effect::IntentPersisted,
    certified_context: CertifiedSideEffectContext,
    evidence_schema_id: SchemaId,
    evidence_hash: ContentDigest,
    replay_verifier_id: Option<events::ReplayVerifierId>,
) -> SideEffectEvidenceReplayRequest {
    SideEffectEvidenceReplayRequest {
        pair_id: intent.pair_id.clone(),
        invocation_epoch: intent.invocation_epoch,
        certified_context,
        evidence_schema_id,
        evidence_hash,
        replay_verifier_id,
    }
}

/// Replay evidence returned for an observed side-effect submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionReplayEvidence {
    /// Submission observed event payload.
    pub submission: side_effect::SubmissionObserved,
    /// Retained submission artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained submission artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay evidence returned for an inconclusive side-effect submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionUnknownReplayEvidence {
    /// Submission-unknown event payload.
    pub unknown: side_effect::SubmissionUnknown,
    /// Retained uncertainty artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained uncertainty artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay evidence returned for a prepared side-effect invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInvocationReplayEvidence {
    /// Invocation prepared event payload.
    pub prepared: side_effect::InvocationPrepared,
    /// Retained prepared invocation artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained prepared invocation artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay evidence returned for a side effect proven not submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSubmittedReplayEvidence {
    /// Not-submitted proof event payload.
    pub proof: side_effect::NotSubmittedProven,
    /// Retained not-submitted proof artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
}

/// Replay evidence returned for an observed side-effect receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptReplayEvidence {
    /// Receipt observed event payload.
    pub receipt: side_effect::ReceiptObserved,
    /// Retained receipt artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained receipt artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay evidence returned for an observed side-effect confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationReplayEvidence {
    /// Confirmation observed event payload.
    pub confirmation: side_effect::ConfirmationObserved,
    /// Retained confirmation artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained confirmation artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay evidence returned for recorded side-effect ambiguity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguityReplayEvidence {
    /// Ambiguous event payload.
    pub ambiguity: side_effect::Ambiguous,
    /// Retained ambiguity artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained ambiguity artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Intent evidence supplied to side-effect replay verifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectIntentReplayEvidence {
    /// Intent persisted event payload.
    pub intent: side_effect::IntentPersisted,
    /// Retained intent artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained intent artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Evidence-only receipt verifier input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectReceiptReplayInput {
    /// Intent evidence for the side effect being verified.
    pub intent: SideEffectIntentReplayEvidence,
    /// Certified submit-node context and output resource authority.
    pub certified_context: CertifiedSideEffectContext,
    /// Prepared invocation evidence, when the side-effect protocol recorded one.
    pub prepared_invocation: Option<PreparedInvocationReplayEvidence>,
    /// Submission evidence when submission was observed before receipt.
    pub submission: Option<SubmissionReplayEvidence>,
    /// Receipt evidence being verified.
    pub receipt: ReceiptReplayEvidence,
}

/// Evidence-only submission verifier input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectSubmissionReplayInput {
    /// Intent evidence for the side effect being verified.
    pub intent: SideEffectIntentReplayEvidence,
    /// Certified submit-node context and output resource authority.
    pub certified_context: CertifiedSideEffectContext,
    /// Prepared invocation evidence, when the side-effect protocol recorded one.
    pub prepared_invocation: Option<PreparedInvocationReplayEvidence>,
    /// Submission evidence being verified.
    pub submission: SubmissionReplayEvidence,
}

/// Evidence-only confirmation verifier input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectConfirmationReplayInput {
    /// Intent evidence for the side effect being verified.
    pub intent: SideEffectIntentReplayEvidence,
    /// Certified submit-node context and output resource authority.
    pub certified_context: CertifiedSideEffectContext,
    /// Prepared invocation evidence, when the side-effect protocol recorded one.
    pub prepared_invocation: Option<PreparedInvocationReplayEvidence>,
    /// Submission evidence when submission was observed before confirmation.
    pub submission: Option<SubmissionReplayEvidence>,
    /// Receipt evidence observed before confirmation.
    pub receipt: Option<ReceiptReplayEvidence>,
    /// Certified verification policy for the side effect.
    pub verification: spec::SideEffectVerificationSpec,
    /// Confirmation evidence being verified.
    pub confirmation: ConfirmationReplayEvidence,
}

/// Evidence-only side-effect replay verifier contract.
///
/// The broker passes only recorded typed events and retained artifact evidence into this trait.
/// Verifier implementations do not receive transports, SDK clients, capability handles, or any
/// generic IO provider from the replay crate.
pub trait SideEffectReplayVerifier {
    /// Replay verifier identity recorded in receipt and confirmation events.
    fn verifier_id(&self) -> &events::ReplayVerifierId;

    /// Verifies submission evidence without live IO.
    fn verify_submission(&self, input: &SideEffectSubmissionReplayInput) -> Result<()>;

    /// Verifies receipt evidence without live IO.
    fn verify_receipt(&self, input: &SideEffectReceiptReplayInput) -> Result<()>;

    /// Verifies confirmation evidence without live IO.
    fn verify_confirmation(&self, input: &SideEffectConfirmationReplayInput) -> Result<()>;
}

/// Request for replaying a retained artifact directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReplayRequest {
    /// Artifact id requested by replay.
    pub artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity expected by replay.
    pub evidence_hash: ContentDigest,
    /// Expected artifact role.
    pub role: ArtifactRole,
    /// Expected content digest.
    pub digest: ContentDigest,
    /// Expected schema id, when schema-bearing.
    pub schema_id: Option<SchemaId>,
    /// Expected semantic type id, when value-bearing.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Expected producer node id, when node-produced.
    pub producer_node_id: Option<NodeId>,
    /// Expected producer seed id, when seed-produced.
    pub producer_seed_id: Option<SeedId>,
}

/// Retained artifact evidence plus bytes returned by replay authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReplayEvidence {
    /// Retained artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained artifact bytes.
    pub artifact_bytes: Vec<u8>,
}

/// Replay-authorized state-output cell artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedCellReplayFrame {
    /// Certified producer node.
    pub node: spec::NodeSpec,
    /// Certified output cell.
    pub cell: spec::CellSpec,
    /// Recorded cell-produced event payload.
    pub produced: events::CellProduced,
    /// Retained state-output artifact evidence.
    pub artifact: StoredArtifactEvidenceRef,
    /// Retained state-output artifact bytes.
    pub artifact_bytes: Vec<u8>,
}
