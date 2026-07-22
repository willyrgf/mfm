use std::collections::BTreeMap;

use mfm_capabilities::CapabilitySetDescriptor;
use mfm_certify::{CertifiedRemediationLink, CertifiedSideEffectContract};
use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
    ContentDigest, DigestAlgorithm, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
    SideEffectPairId, SpecHash, StateKind, StateVersion,
};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualResolutionEvidenceRef,
    ManualResolutionPrefixAuthority, ManualResolutionProofAuthority,
    VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1::{self as spec, CanonicalizerIdentity, HashedSpecEnvelope};
use mfm_spec::SpecError;
use mfm_store::v1::{
    self as store, ArtifactEvidenceRef as StoredArtifactEvidenceRef, KernelEventEnvelope,
    ProjectionSnapshot,
};

#[path = "verification_helpers.rs"]
mod verification_helpers;
use self::verification_helpers::*;
#[path = "value_read.rs"]
mod value_read;
pub use self::value_read::{
    canonical_value_bytes, decode_produced_value, external_read_evidence,
    fact_query_evidence_for_attempt, load_node_config, load_node_context, load_node_input,
    produced_input_frames, single_state_output_frame, verify_external_read_state,
    verify_pure_state, verify_recorded_fact_batch_evidence, verify_recorded_fact_evidence,
};
#[path = "broker.rs"]
mod broker;
#[path = "evidence.rs"]
mod evidence;
pub use self::evidence::{
    AmbiguityReplayEvidence, ArtifactReplayEvidence, ArtifactReplayRequest,
    CertifiedSideEffectContext, ConfirmationReplayEvidence, NotSubmittedReplayEvidence,
    PreparedInvocationReplayEvidence, ProducedCellReplayFrame, ReceiptReplayEvidence,
    RetainedSourceFactReplayEvent, SideEffectConfirmationReplayInput,
    SideEffectEvidenceReplayRequest, SideEffectIntentReplayEvidence, SideEffectReceiptReplayInput,
    SideEffectReplayFrame, SideEffectReplayVerifier, SideEffectSubmissionReplayInput,
    SubmissionReplayEvidence, SubmissionUnknownReplayEvidence,
};

/// Result type for replay broker operations.
pub type Result<T> = std::result::Result<T, ReplayError>;

/// Typed replay validation error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", .kind.code())]
pub struct ReplayError {
    /// Stable replay error category.
    pub kind: ReplayErrorKind,
    /// Redaction-safe diagnostic.
    pub message: String,
}

impl ReplayError {
    /// Creates a replay error with a stable category.
    pub fn new(kind: ReplayErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable replay error code.
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl From<SpecError> for ReplayError {
    fn from(error: SpecError) -> Self {
        Self::new(ReplayErrorKind::CertifiedSpec, error.to_string())
    }
}

impl From<store::StoreError> for ReplayError {
    fn from(error: store::StoreError) -> Self {
        Self::new(ReplayErrorKind::InvalidRunStream, error.to_string())
    }
}

impl From<mfm_runtime::RuntimeError> for ReplayError {
    fn from(error: mfm_runtime::RuntimeError) -> Self {
        Self::new(
            ReplayErrorKind::CertifiedEvidenceMismatch,
            error.to_string(),
        )
    }
}

/// Stable typed replay error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReplayErrorKind {
    /// The hash-only spec envelope is invalid.
    CertifiedSpec,
    /// No authoritative run-start event was present.
    RunAdmittedMissing,
    /// The run stream is not a valid store-owned typed stream.
    InvalidRunStream,
    /// The stream is bound to a different certified spec hash.
    SpecHashMismatch,
    /// The stream or authority carries a different canonicalizer identity.
    CanonicalizerMismatch,
    /// The run-start descriptor identities disagree with the certified spec.
    DescriptorIdentityMismatch,
    /// Runner executable identities disagree with replay authority.
    ExecutableIdentityMismatch,
    /// Adapter executable identities disagree with replay authority.
    AdapterExecutableMismatch,
    /// A requested node is not certified to use the requested capability.
    UnsupportedCapability,
    /// A requested node is not certified to use the requested adapter.
    UnsupportedAdapter,
    /// Replay attempted to request live capability access.
    LiveCapabilityRequest,
    /// A requested recorded fact was absent.
    FactMissing,
    /// A recorded fact did not match the replay request.
    FactMismatch,
    /// Required retained artifact evidence was absent.
    ArtifactMissing,
    /// Retained artifact evidence disagreed with typed event evidence.
    ArtifactMismatch,
    /// Required side-effect evidence was absent.
    SideEffectMissing,
    /// Recorded side-effect evidence did not match the replay request.
    SideEffectMismatch,
    /// A stream event references evidence outside the certified spec.
    CertifiedEvidenceMismatch,
    /// A side-effect replay verifier identity did not match.
    ReplayVerifierMismatch,
}

impl ReplayErrorKind {
    /// Returns the stable replay error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CertifiedSpec => "MFM_REPLAY_CERTIFIED_SPEC_INVALID",
            Self::RunAdmittedMissing => "MFM_REPLAY_RUN_ADMITTED_MISSING",
            Self::InvalidRunStream => "MFM_REPLAY_STREAM_INVALID",
            Self::SpecHashMismatch => "MFM_REPLAY_SPEC_HASH_MISMATCH",
            Self::CanonicalizerMismatch => "MFM_REPLAY_CANONICALIZER_MISMATCH",
            Self::DescriptorIdentityMismatch => "MFM_REPLAY_DESCRIPTOR_IDENTITY_MISMATCH",
            Self::ExecutableIdentityMismatch => "MFM_REPLAY_EXECUTABLE_IDENTITY_MISMATCH",
            Self::AdapterExecutableMismatch => "MFM_REPLAY_ADAPTER_EXECUTABLE_MISMATCH",
            Self::UnsupportedCapability => "MFM_REPLAY_CAPABILITY_UNSUPPORTED",
            Self::UnsupportedAdapter => "MFM_REPLAY_ADAPTER_UNSUPPORTED",
            Self::LiveCapabilityRequest => "MFM_REPLAY_LIVE_CAPABILITY_REQUEST",
            Self::FactMissing => "MFM_REPLAY_FACT_MISSING",
            Self::FactMismatch => "MFM_REPLAY_FACT_MISMATCH",
            Self::ArtifactMissing => "MFM_REPLAY_ARTIFACT_MISSING",
            Self::ArtifactMismatch => "MFM_REPLAY_ARTIFACT_MISMATCH",
            Self::SideEffectMissing => "MFM_REPLAY_SIDE_EFFECT_MISSING",
            Self::SideEffectMismatch => "MFM_REPLAY_SIDE_EFFECT_MISMATCH",
            Self::CertifiedEvidenceMismatch => "MFM_REPLAY_CERTIFIED_EVIDENCE_MISMATCH",
            Self::ReplayVerifierMismatch => "MFM_REPLAY_VERIFIER_MISMATCH",
        }
    }
}

/// Sealed replay read authority minted from certified spec and verified run history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReadAuthority {
    certified_spec: HashedSpecEnvelope,
    stream: Vec<KernelEventEnvelope>,
    canonicalizer_identity: CanonicalizerIdentity,
    runner_executables: Vec<events::ExecutableIdentity>,
    adapter_executables: Vec<events::ExecutableIdentity>,
    capability_implementations: Vec<events::CapabilityImplementationIdentity>,
    artifact_evidence: Vec<StoredArtifactEvidenceRef>,
    artifact_bytes: BTreeMap<ReplayArtifactAuthorityKey, Vec<u8>>,
    additional_artifact_evidence: Vec<StoredArtifactEvidenceRef>,
    source_fact_events: Vec<RetainedSourceFactReplayEvent>,
}

impl ReplayReadAuthority {
    /// Mints replay read authority with retained source facts and additional certified artifacts.
    pub fn from_verified_run_history_view_with_source_facts_and_artifacts(
        runtime_spec: &mfm_runtime::CertifiedRuntimeSpec,
        verified_view: &mfm_runtime::VerifiedRunHistoryView,
        source_fact_events: Vec<RetainedSourceFactReplayEvent>,
        additional_artifacts: Vec<store::VerifiedRetainedArtifactBytes>,
    ) -> Result<Self> {
        if runtime_spec.spec_hash() != verified_view.spec_hash() {
            return Err(ReplayError::new(
                ReplayErrorKind::SpecHashMismatch,
                "verified history spec hash does not match certified runtime spec",
            ));
        }
        let run_admitted = verified_view.run_admitted();
        let mut artifact_evidence = verified_view
            .artifact_store()
            .artifacts()
            .map(|(_, artifact)| artifact.evidence().clone())
            .collect::<Vec<_>>();
        let mut artifact_bytes = verified_view
            .artifact_store()
            .artifacts()
            .map(|(_, artifact)| {
                Ok((
                    replay_artifact_authority_key(artifact.evidence())?,
                    artifact.bytes().to_vec(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let artifacts = artifact_map(artifact_evidence.clone())?;
        verify_replay_artifact_authority(verified_view, &artifacts)?;
        let additional_artifact_evidence = additional_artifacts
            .iter()
            .map(|artifact| artifact.evidence().clone())
            .collect::<Vec<_>>();
        artifact_evidence.extend(
            additional_artifacts
                .iter()
                .map(|artifact| artifact.evidence().clone()),
        );
        for artifact in additional_artifacts {
            let key = replay_artifact_authority_key(artifact.evidence())?;
            artifact_bytes.push((key, artifact.into_bytes()));
        }
        let artifact_bytes = artifact_bytes_map(artifact_bytes)?;
        Ok(Self {
            certified_spec: runtime_spec.envelope().clone(),
            stream: verified_view.events().to_vec(),
            canonicalizer_identity: runtime_spec
                .envelope()
                .spec
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            runner_executables: run_admitted.runner_executables.clone(),
            adapter_executables: run_admitted.adapter_executables.clone(),
            capability_implementations: run_admitted.capability_implementations.clone(),
            artifact_evidence,
            artifact_bytes,
            additional_artifact_evidence,
            source_fact_events,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ArtifactEvidenceExpectation<'a> {
    artifact_id: &'a ArtifactId,
    evidence_hash: &'a ContentDigest,
    digest: &'a ContentDigest,
    schema_id: Option<&'a SchemaId>,
    semantic_type_id: Option<&'a SemanticTypeId>,
    role: ArtifactRole,
    producer_node_id: Option<&'a NodeId>,
    producer_seed_id: Option<&'a SeedId>,
}

type FactReplayKey = mfm_facts::FactClaimId;
type ReplayArtifactAuthorityKey = (ArtifactId, ContentDigest);
type SideEffectKey = (SideEffectPairId, u32);

trait SideEffectReplayArtifact {
    fn artifact_id(&self) -> &ArtifactId;
    fn evidence_hash(&self) -> &ContentDigest;
    fn artifact_evidence_hash(&self) -> &ContentDigest;
    fn evidence_schema_id(&self) -> &SchemaId;
    fn artifact_role(&self) -> ArtifactRole;
    fn producer_node_id(&self) -> &NodeId;
    fn mismatch_message(&self) -> &'static str;
    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        None
    }
}

impl SideEffectReplayArtifact for side_effect::SubmissionObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.submission_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.submission_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.submission_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.submission_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Submission
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "submission evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::SubmissionUnknown {
    fn artifact_id(&self) -> &ArtifactId {
        &self.evidence_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.evidence_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.evidence_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::SubmissionUnknownEvidence
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "submission-unknown evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::NotSubmittedProven {
    fn artifact_id(&self) -> &ArtifactId {
        &self.proof_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.proof_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.proof_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.proof_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::NotSubmittedProof
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "not-submitted proof evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::ReceiptObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.receipt_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.receipt_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.receipt_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.receipt_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Receipt
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "receipt evidence mismatch"
    }

    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        Some(&self.replay_verifier_id)
    }
}

impl SideEffectReplayArtifact for side_effect::ConfirmationObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.confirmation_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.confirmation_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.confirmation_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.confirmation_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Confirmation
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "confirmation evidence mismatch"
    }

    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        Some(&self.replay_verifier_id)
    }
}

impl SideEffectReplayArtifact for side_effect::Ambiguous {
    fn artifact_id(&self) -> &ArtifactId {
        &self.evidence_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.evidence_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.evidence_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::AmbiguityEvidence
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "ambiguity evidence mismatch"
    }
}

/// Evidence-only broker for certified typed replay.
#[derive(Debug, Clone)]
pub struct ReplayBroker {
    certified_spec: HashedSpecEnvelope,
    stream: Vec<KernelEventEnvelope>,
    run_id: events::RunAdmitted,
    projection: ProjectionSnapshot,
    retained_artifacts: BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    artifact_bytes: BTreeMap<ReplayArtifactAuthorityKey, Vec<u8>>,
    artifact_byte_authority: store::ArtifactByteAuthorityMap,
    artifacts: BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    facts: BTreeMap<FactReplayKey, events::FactRecorded>,
    fact_events: BTreeMap<FactReplayKey, KernelEventEnvelope>,
    intents: BTreeMap<SideEffectPairId, side_effect::IntentPersisted>,
    prepared_invocations: BTreeMap<SideEffectKey, side_effect::InvocationPrepared>,
    submissions: BTreeMap<SideEffectKey, side_effect::SubmissionObserved>,
    submission_unknown: BTreeMap<SideEffectKey, side_effect::SubmissionUnknown>,
    not_submitted: BTreeMap<SideEffectKey, side_effect::NotSubmittedProven>,
    receipts: BTreeMap<SideEffectKey, side_effect::ReceiptObserved>,
    confirmations: BTreeMap<SideEffectKey, side_effect::ConfirmationObserved>,
    ambiguities: BTreeMap<SideEffectKey, side_effect::Ambiguous>,
    manual_resolutions: BTreeMap<RunId, VerifiedManualResolutionForPrefix>,
}

#[cfg(test)]
mod tests;
